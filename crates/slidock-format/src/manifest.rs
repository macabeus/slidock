//! `presentation.json`: a document set's order and metadata, in plain types independent of the IR. `--emit documents`
//! prints it with [`serialize_manifest`] (canonical JSON), so the same deck always writes the same bytes; it holds no
//! timestamps, absolute paths or diagnostics. [`read_manifest`] checks a manifest's text against the format's strict
//! schema and returns every issue, each with its path.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use slidock_canon::{Json, to_js_string, utf16, write_json_string};
use thiserror::Error;

use crate::paths::{is_sha256, layout_path, slide_path};

/// The most slides a deck holds (the compiler's limit).
pub const MAX_SLIDES: usize = 300;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Manifest {
    pub title: String,
    pub size: ManifestSize,
    /// Show order, hidden slides included.
    pub slides: Vec<ManifestSlide>,
    pub layouts: Vec<ManifestLayout>,
    pub themes: BTreeMap<String, ManifestTheme>,
    /// The file the deck was compiled from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ManifestSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ManifestSize {
    pub w: f64,
    pub h: f64,
    pub unit: SizeUnit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeUnit {
    #[default]
    Pt,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSlide {
    pub id: String,
    /// `slides/<fileId>.html`.
    pub file: String,
    /// For navigation; empty when the slide has none.
    pub title: String,
    pub hidden: bool,
    /// The layout the slide document names; its file holds what shows behind the slide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Speaker notes as plain text, paragraphs separated by `\n`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<ManifestTransition>,
    /// Set when the slide failed to compile: its document shows a marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestTransition {
    /// An opaque `<namespace>:<name>` effect id.
    pub effect: String,
    /// What a player plays for an effect it lacks.
    pub fallback: TransitionFallback,
    pub duration_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advance: Option<Advance>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionFallback {
    None,
    Dissolve,
    Push,
    Wipe,
}

impl TransitionFallback {
    const ALL: [(TransitionFallback, &'static str); 4] = [
        (TransitionFallback::None, "none"),
        (TransitionFallback::Dissolve, "dissolve"),
        (TransitionFallback::Push, "push"),
        (TransitionFallback::Wipe, "wipe"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Advance {
    pub on_click: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ManifestLayout {
    pub id: String,
    /// `layouts/<fileId>.html`.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The theme the layout's colours come from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ManifestTheme {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub colors: SchemeColors,
}

macro_rules! scheme_colors {
    ($($field:ident = $slot:literal),* $(,)?) => {
        /// A theme's colour per scheme slot, every slot present: `#rrggbb`, `#rrggbbaa` or a Display P3 colour.
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct SchemeColors {
            $(#[serde(rename = $slot)] pub $field: String,)*
        }

        impl SchemeColors {
            /// The scheme slots, in the format's order.
            pub const SLOTS: &'static [&'static str] = &[$($slot),*];

            /// Every slot set to `color`.
            pub fn uniform(color: &str) -> SchemeColors {
                SchemeColors { $($field: color.to_owned(),)* }
            }

            fn read(reader: &mut Reader, value: &Value, path: &Path) -> Option<SchemeColors> {
                let object = reader.object(value, path, Self::SLOTS)?;
                $(let $field = reader.required(object, path, $slot, |r, v, p| r.string(v, p, Some(is_color), "a colour"));)*
                Some(SchemeColors { $($field: $field?,)* })
            }
        }
    };
}

scheme_colors! {
    dk1 = "dk1", lt1 = "lt1", dk2 = "dk2", lt2 = "lt2",
    accent1 = "accent1", accent2 = "accent2", accent3 = "accent3",
    accent4 = "accent4", accent5 = "accent5", accent6 = "accent6",
    hlink = "hlink", fol_hlink = "folHlink",
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ManifestSource {
    pub format: String,
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Generator {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ManifestError {
    /// The text is not JSON.
    #[error("{0}")]
    NotJson(String),
    /// The JSON breaks the schema, or its entries disagree with each other.
    #[error("{0}")]
    Invalid(String),
}

/// Reads a manifest's text, returning every failure rather than stopping at the first.
pub fn read_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let json: Value =
        serde_json::from_str(text).map_err(|error| ManifestError::NotJson(error.to_string()))?;
    let mut reader = Reader::default();
    let manifest = read_root(&mut reader, &json);
    match manifest {
        Some(manifest) if reader.issues.is_empty() => {
            let inconsistent = manifest_consistency(&manifest);
            if inconsistent.is_empty() {
                Ok(manifest)
            } else {
                Err(ManifestError::Invalid(inconsistent.join("; ")))
            }
        }
        _ => Err(ManifestError::Invalid(reader.issues.join("; "))),
    }
}

/// The manifest's text, after checking it: canonical JSON's layout (keys sorted by UTF-16 code unit, two-space indent,
/// a trailing newline), with each number as `JSON.stringify` prints it, so a size below 1e-6, which the schema
/// accepts, prints in e-notation (`1e-7`).
pub fn serialize_manifest(manifest: &Manifest) -> Result<String, ManifestError> {
    let json = Json::from_serialize(manifest)
        .map_err(|error| ManifestError::Invalid(error.to_string()))?;
    let mut text = String::new();
    print_json(&json, 0, &mut text);
    text.push('\n');
    read_manifest(&text)?;
    Ok(text)
}

fn print_json(value: &Json, depth: usize, out: &mut String) {
    let pad = |depth: usize, out: &mut String| out.push_str(&"  ".repeat(depth));
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => out.push_str(&to_js_string(*n)),
        Json::String(s) => write_json_string(s, out),
        Json::Array(items) if items.is_empty() => out.push_str("[]"),
        Json::Object(members) if members.is_empty() => out.push_str("{}"),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                pad(depth + 1, out);
                print_json(item, depth + 1, out);
            }
            out.push('\n');
            pad(depth, out);
            out.push(']');
        }
        Json::Object(members) => {
            let mut members: Vec<&(String, Json)> = members.iter().collect();
            members.sort_by(|a, b| utf16::cmp(&a.0, &b.0));
            out.push('{');
            for (i, (key, item)) in members.into_iter().enumerate() {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                pad(depth + 1, out);
                write_json_string(key, out);
                out.push_str(": ");
                print_json(item, depth + 1, out);
            }
            out.push('\n');
            pad(depth, out);
            out.push('}');
        }
    }
}

/// Why a manifest's entries disagree with each other, beyond the schema; empty when they agree.
pub fn manifest_consistency(manifest: &Manifest) -> Vec<String> {
    let mut problems = Vec::new();
    for slide in &manifest.slides {
        let expected = slide_path(&slide.id);
        if slide.file != expected {
            problems.push(format!("slide {} must be in {expected}", slide.id));
        }
    }
    for layout in &manifest.layouts {
        let expected = layout_path(&layout.id);
        if layout.file != expected {
            problems.push(format!("layout {} must be in {expected}", layout.id));
        }
        if let Some(theme) = &layout.theme
            && !manifest.themes.contains_key(theme)
        {
            problems.push(format!(
                "layout {} names the theme {theme}, which the manifest does not hold",
                layout.id
            ));
        }
    }
    problems
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`
fn is_id(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && text.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `^[A-Z]{2,4}\d{4}$`: a diagnostic code.
fn is_code(text: &str) -> bool {
    let letters = text.bytes().take_while(u8::is_ascii_uppercase).count();
    (2..=4).contains(&letters)
        && text.len() == letters + 4
        && text.bytes().skip(letters).all(|b| b.is_ascii_digit())
}

/// `^[a-z]+:[!-~]{1,120}$`: an opaque effect id.
fn is_effect(text: &str) -> bool {
    let Some((namespace, name)) = text.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && namespace.bytes().all(|b| b.is_ascii_lowercase())
        && (1..=120).contains(&name.len())
        && name.bytes().all(|b| (b'!'..=b'~').contains(&b))
}

/// `#rrggbb`, `#rrggbbaa` or `color(display-p3 r g b[ / a])` with `[0-9.]+` components.
fn is_color(text: &str) -> bool {
    let lower_hex = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if let Some(hex) = text.strip_prefix('#') {
        return (hex.len() == 6 || hex.len() == 8) && lower_hex(hex);
    }
    let Some(inner) = text
        .strip_prefix("color(display-p3 ")
        .and_then(|s| s.strip_suffix(')'))
    else {
        return false;
    };
    let component = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let (rgb, alpha) = match inner.split_once(" / ") {
        Some((rgb, alpha)) => (rgb, Some(alpha)),
        None => (inner, None),
    };
    let parts: Vec<&str> = rgb.split(' ').collect();
    parts.len() == 3 && parts.iter().all(|part| component(part)) && alpha.is_none_or(component)
}

/// `^[a-z]{1,16}$`
fn is_format_name(text: &str) -> bool {
    (1..=16).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_lowercase())
}

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const MAX_MS: f64 = 3_600_000.0;

type Path = Vec<String>;

fn child(path: &Path, key: &str) -> Path {
    let mut next = path.clone();
    next.push(key.to_owned());
    next
}

/// A schema check over parsed JSON that records each issue with its dotted path, as zod reports them.
#[derive(Default)]
struct Reader {
    issues: Vec<String>,
}

type Check = fn(&str) -> bool;

impl Reader {
    fn issue(&mut self, path: &Path, message: impl Into<String>) {
        let message = message.into();
        if path.is_empty() {
            self.issues.push(message);
        } else {
            self.issues.push(format!("{}: {message}", path.join(".")));
        }
    }

    /// An object with no keys outside `keys`.
    fn object<'v>(
        &mut self,
        value: &'v Value,
        path: &Path,
        keys: &[&str],
    ) -> Option<&'v Map<String, Value>> {
        let Some(object) = value.as_object() else {
            self.issue(
                path,
                format!("Invalid input: expected object, received {}", kind(value)),
            );
            return None;
        };
        for key in object.keys() {
            if !keys.contains(&key.as_str()) {
                self.issue(path, format!("Unrecognized key: \"{key}\""));
            }
        }
        Some(object)
    }

    fn required<T>(
        &mut self,
        object: &Map<String, Value>,
        path: &Path,
        key: &str,
        read: impl FnOnce(&mut Reader, &Value, &Path) -> Option<T>,
    ) -> Option<T> {
        let path = child(path, key);
        match object.get(key) {
            Some(value) => read(self, value, &path),
            None => {
                self.issue(&path, "Invalid input: expected a value, received undefined");
                None
            }
        }
    }

    /// `Some(None)` when absent, `None` when present and invalid.
    fn optional<T>(
        &mut self,
        object: &Map<String, Value>,
        path: &Path,
        key: &str,
        read: impl FnOnce(&mut Reader, &Value, &Path) -> Option<T>,
    ) -> Option<Option<T>> {
        match object.get(key) {
            Some(value) => read(self, value, &child(path, key)).map(Some),
            None => Some(None),
        }
    }

    fn string(
        &mut self,
        value: &Value,
        path: &Path,
        check: Option<Check>,
        what: &str,
    ) -> Option<String> {
        let Some(text) = value.as_str() else {
            self.issue(
                path,
                format!("Invalid input: expected string, received {}", kind(value)),
            );
            return None;
        };
        if check.is_some_and(|check| !check(text)) {
            self.issue(path, format!("Invalid string: expected {what}"));
            return None;
        }
        Some(text.to_owned())
    }

    /// A string of `min..=max` UTF-16 code units.
    fn bounded(&mut self, value: &Value, path: &Path, min: usize, max: usize) -> Option<String> {
        let text = self.string(value, path, None, "")?;
        let len = slidock_canon::utf16::len(&text);
        if len < min || len > max {
            self.issue(
                path,
                format!("Invalid string: expected {min} to {max} characters"),
            );
            return None;
        }
        Some(text)
    }

    fn id(&mut self, value: &Value, path: &Path) -> Option<String> {
        self.string(value, path, Some(is_id), "an id")
    }

    fn boolean(&mut self, value: &Value, path: &Path) -> Option<bool> {
        let flag = value.as_bool();
        if flag.is_none() {
            self.issue(
                path,
                format!("Invalid input: expected boolean, received {}", kind(value)),
            );
        }
        flag
    }

    fn number(&mut self, value: &Value, path: &Path) -> Option<f64> {
        let number = value.as_f64();
        if number.is_none() {
            self.issue(
                path,
                format!("Invalid input: expected number, received {}", kind(value)),
            );
        }
        number
    }

    /// A safe integer in `min..=max`.
    fn integer(&mut self, value: &Value, path: &Path, max: f64) -> Option<f64> {
        let number = self.number(value, path)?;
        if number.fract() != 0.0 || number.abs() > MAX_SAFE_INTEGER {
            self.issue(path, "Invalid input: expected int, received number");
            return None;
        }
        if !(0.0..=max).contains(&number) {
            self.issue(
                path,
                format!("Too big or too small: expected an integer from 0 to {max}"),
            );
            return None;
        }
        Some(number)
    }

    fn ms(&mut self, value: &Value, path: &Path) -> Option<u32> {
        // In 0..=3_600_000, so the conversion is exact.
        self.integer(value, path, MAX_MS).map(|ms| ms as u32)
    }

    fn array<T>(
        &mut self,
        value: &Value,
        path: &Path,
        mut read: impl FnMut(&mut Reader, &Value, &Path) -> Option<T>,
    ) -> Option<Vec<T>> {
        let Some(items) = value.as_array() else {
            self.issue(
                path,
                format!("Invalid input: expected array, received {}", kind(value)),
            );
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        let mut ok = true;
        for (index, item) in items.iter().enumerate() {
            match read(self, item, &child(path, &index.to_string())) {
                Some(item) => out.push(item),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn read_root(r: &mut Reader, value: &Value) -> Option<Manifest> {
    let root = Path::new();
    let keys = [
        "title",
        "size",
        "slides",
        "layouts",
        "themes",
        "source",
        "generator",
    ];
    let object = r.object(value, &root, &keys)?;
    let title = r.required(object, &root, "title", |r, v, p| r.bounded(v, p, 0, 256));
    let size = r.required(object, &root, "size", read_size);
    let slides = r.required(object, &root, "slides", |r, v, p| r.array(v, p, read_slide));
    let layouts = r.required(object, &root, "layouts", |r, v, p| {
        r.array(v, p, read_layout)
    });
    let themes = r.required(object, &root, "themes", read_themes);
    let source = r.optional(object, &root, "source", read_source);
    let generator = r.optional(object, &root, "generator", read_generator);
    Some(Manifest {
        title: title?,
        size: size?,
        slides: slides?,
        layouts: layouts?,
        themes: themes?,
        source: source?,
        generator: generator?,
    })
}

fn read_size(r: &mut Reader, value: &Value, path: &Path) -> Option<ManifestSize> {
    let object = r.object(value, path, &["w", "h", "unit"])?;
    let side = |r: &mut Reader, v: &Value, p: &Path| {
        let n = r.number(v, p)?;
        if n > 0.0 && n <= 100_000.0 {
            Some(n)
        } else {
            r.issue(p, "Too big or too small: expected a number in (0, 100000]");
            None
        }
    };
    let w = r.required(object, path, "w", side);
    let h = r.required(object, path, "h", side);
    let unit = r.required(object, path, "unit", |r, v, p| {
        (v.as_str() == Some("pt"))
            .then_some(SizeUnit::Pt)
            .or_else(|| {
                r.issue(p, "Invalid input: expected \"pt\"");
                None
            })
    });
    Some(ManifestSize {
        w: w?,
        h: h?,
        unit: unit?,
    })
}

fn read_slide(r: &mut Reader, value: &Value, path: &Path) -> Option<ManifestSlide> {
    let keys = [
        "id",
        "file",
        "title",
        "hidden",
        "layout",
        "notes",
        "transition",
        "failed",
    ];
    let object = r.object(value, path, &keys)?;
    let id = r.required(object, path, "id", Reader::id);
    let file = r.required(object, path, "file", |r, v, p| r.string(v, p, None, ""));
    let title = r.required(object, path, "title", |r, v, p| r.bounded(v, p, 0, 256));
    let hidden = r.required(object, path, "hidden", Reader::boolean);
    let layout = r.optional(object, path, "layout", Reader::id);
    let notes = r.optional(object, path, "notes", |r, v, p| r.bounded(v, p, 0, 65_536));
    let transition = r.optional(object, path, "transition", read_transition);
    let failed = r.optional(object, path, "failed", |r, v, p| {
        r.string(v, p, Some(is_code), "a diagnostic code")
    });
    Some(ManifestSlide {
        id: id?,
        file: file?,
        title: title?,
        hidden: hidden?,
        layout: layout?,
        notes: notes?,
        transition: transition?,
        failed: failed?,
    })
}

fn read_transition(r: &mut Reader, value: &Value, path: &Path) -> Option<ManifestTransition> {
    let object = r.object(
        value,
        path,
        &["effect", "fallback", "durationMs", "advance"],
    )?;
    let effect = r.required(object, path, "effect", |r, v, p| {
        r.string(v, p, Some(is_effect), "an effect id")
    });
    let fallback = r.required(object, path, "fallback", |r, v, p| {
        let found = TransitionFallback::ALL
            .iter()
            .find(|(_, name)| v.as_str() == Some(*name))
            .map(|(fallback, _)| *fallback);
        if found.is_none() {
            r.issue(
                p,
                "Invalid option: expected one of \"none\"|\"dissolve\"|\"push\"|\"wipe\"",
            );
        }
        found
    });
    let duration_ms = r.required(object, path, "durationMs", Reader::ms);
    let advance = r.optional(object, path, "advance", |r, v, p| {
        let object = r.object(v, p, &["onClick", "afterMs"])?;
        let on_click = r.required(object, p, "onClick", Reader::boolean);
        let after_ms = r.optional(object, p, "afterMs", Reader::ms);
        Some(Advance {
            on_click: on_click?,
            after_ms: after_ms?,
        })
    });
    Some(ManifestTransition {
        effect: effect?,
        fallback: fallback?,
        duration_ms: duration_ms?,
        advance: advance?,
    })
}

fn read_layout(r: &mut Reader, value: &Value, path: &Path) -> Option<ManifestLayout> {
    let object = r.object(value, path, &["id", "file", "name", "theme", "failed"])?;
    let id = r.required(object, path, "id", Reader::id);
    let file = r.required(object, path, "file", |r, v, p| r.string(v, p, None, ""));
    let name = r.optional(object, path, "name", |r, v, p| r.bounded(v, p, 0, 256));
    let theme = r.optional(object, path, "theme", Reader::id);
    let failed = r.optional(object, path, "failed", |r, v, p| {
        r.string(v, p, Some(is_code), "a diagnostic code")
    });
    Some(ManifestLayout {
        id: id?,
        file: file?,
        name: name?,
        theme: theme?,
        failed: failed?,
    })
}

fn read_themes(
    r: &mut Reader,
    value: &Value,
    path: &Path,
) -> Option<BTreeMap<String, ManifestTheme>> {
    let Some(object) = value.as_object() else {
        r.issue(
            path,
            format!("Invalid input: expected record, received {}", kind(value)),
        );
        return None;
    };
    let mut themes = BTreeMap::new();
    let mut ok = true;
    for (id, theme) in object {
        let theme_path = child(path, id);
        if !is_id(id) {
            r.issue(&theme_path, "Invalid key in record: expected an id");
            ok = false;
            continue;
        }
        let read = (|| {
            let object = r.object(theme, &theme_path, &["name", "colors"])?;
            let name = r.optional(object, &theme_path, "name", |r, v, p| {
                r.bounded(v, p, 0, 256)
            });
            let colors = r.required(object, &theme_path, "colors", SchemeColors::read);
            Some(ManifestTheme {
                name: name?,
                colors: colors?,
            })
        })();
        match read {
            Some(theme) => {
                themes.insert(id.clone(), theme);
            }
            None => ok = false,
        }
    }
    ok.then_some(themes)
}

fn read_source(r: &mut Reader, value: &Value, path: &Path) -> Option<ManifestSource> {
    let object = r.object(value, path, &["format", "name", "sha256", "bytes"])?;
    let format = r.required(object, path, "format", |r, v, p| {
        r.string(v, p, Some(is_format_name), "a format name")
    });
    let name = r.required(object, path, "name", |r, v, p| r.bounded(v, p, 1, 256));
    let sha256 = r.required(object, path, "sha256", |r, v, p| {
        r.string(v, p, Some(is_sha256), "a SHA-256")
    });
    // A safe integer, so the conversion is exact.
    let bytes = r.required(object, path, "bytes", |r, v, p| {
        r.integer(v, p, MAX_SAFE_INTEGER).map(|n| n as u64)
    });
    Some(ManifestSource {
        format: format?,
        name: name?,
        sha256: sha256?,
        bytes: bytes?,
    })
}

fn read_generator(r: &mut Reader, value: &Value, path: &Path) -> Option<Generator> {
    let object = r.object(value, path, &["name", "version"])?;
    let name = r.required(object, path, "name", |r, v, p| r.bounded(v, p, 1, 128));
    let version = r.required(object, path, "version", |r, v, p| r.bounded(v, p, 1, 64));
    Some(Generator {
        name: name?,
        version: version?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_string_patterns() {
        assert!(
            is_code("IR0908")
                && is_code("HTML6900")
                && !is_code("I0908")
                && !is_code("IR090")
                && !is_code("ABCDE1234")
        );
        assert!(is_effect("pptx:fade") && !is_effect("keynote:apple:move in") && is_effect("a:!~"));
        assert!(!is_effect("pptx:") && !is_effect(":x") && !is_effect("Pptx:x"));
        assert!(
            is_color("#a1b2c3")
                && is_color("#a1b2c3d4")
                && !is_color("#A1B2C3")
                && !is_color("#abc")
        );
        assert!(
            is_id("ir-S2") && is_id(&"a".repeat(128)) && !is_id(&"a".repeat(129)) && !is_id("_x")
        );
    }
}
