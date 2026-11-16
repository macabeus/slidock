//! What the lint reports, as data. An error means a surface would show the deck wrongly or unsafely, and the compiler
//! fails the container whose file holds it; a warning means the file set carries something nothing uses.

use std::fmt;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProblemSeverity {
    Error,
    Warning,
}

macro_rules! problem_codes {
    ($($variant:ident = $name:literal, $severity:ident, $template:literal;)*) => {
        /// A lint rule.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
        pub enum ProblemCode {
            $(#[serde(rename = $name)] $variant,)*
        }

        impl ProblemCode {
            pub const ALL: &'static [ProblemCode] = &[$(ProblemCode::$variant,)*];

            /// The kebab-case name the reports carry.
            pub const fn name(self) -> &'static str {
                match self { $(ProblemCode::$variant => $name,)* }
            }

            pub const fn severity(self) -> ProblemSeverity {
                match self { $(ProblemCode::$variant => ProblemSeverity::$severity,)* }
            }

            /// The message, with `{key}` placeholders for the problem's data.
            pub const fn template(self) -> &'static str {
                match self { $(ProblemCode::$variant => $template,)* }
            }
        }
    };
}

problem_codes! {
    // The manifest and the file set.
    ManifestMissing = "manifest-missing", Error, "presentation.json is missing";
    ManifestInvalid = "manifest-invalid", Error, "presentation.json is invalid: {detail}";
    DuplicateId = "duplicate-id", Error, "Id {id} is listed more than once";
    TooManySlides = "too-many-slides", Error, "The presentation has {count} slides; at most {max} are supported";
    FileMissing = "file-missing", Error, "{file} is listed in presentation.json but is not in the file set";
    UnlistedDocument = "unlisted-document", Warning, "{file} is not listed in presentation.json";
    OrphanFile = "orphan-file", Warning, "{file} is not referenced by any document";
    // One document.
    DocumentRoot = "document-root", Error, "{file}: {detail}";
    LayoutMissing = "layout-missing", Error, "{file} names the layout {layout}, which has no layout file";
    Script = "script", Error, "{file} has a <script> element";
    ElementNotAllowed = "element-not-allowed", Error,
        "{file} has a <{element}>; documents hold text, media and drawing, nothing that runs, embeds or navigates";
    EventHandler = "event-handler", Error, "{file} has an inline event handler ({attribute})";
    JavascriptUrl = "javascript-url", Error, "{file} has a javascript: URL";
    LinkScheme = "link-scheme", Error, "{file} links to {url}; a link is a fragment or an http, https or mailto URL";
    ResetStyle = "reset-style", Error, "{file}: {detail}";
    ExtraStyle = "extra-style", Error, "{file} has a <{element}>; everything but the reset rule is inline";
    StyleInvalid = "style-invalid", Error, "{file}: a style attribute does not parse ({detail})";
    ElementUnpositioned = "element-unpositioned", Error,
        "The element {id} in {file} lacks {missing}; elements are placed absolutely in the slide box";
    DuplicateElement = "duplicate-element", Error, "data-el {id} appears more than once in {file} {where}";
    MediaReference = "media-reference", Error,
        "{file} references {reference}; documents reference media as ../media/<sha256>.<ext>";
    MediaMissing = "media-missing", Error, "{file} references {reference}, which is not in the file set";
    RemoteUrl = "remote-url", Error, "{file} loads {reference}; documents load only their own media";
    FixedPosition = "fixed-position", Error, "{file} uses position: fixed; elements stay in the slide box";
    ViewportUnits = "viewport-units", Error,
        "{file} uses viewport units, which follow the window rather than the slide";
    VideoPoster = "video-poster", Error,
        "{file} has a <video> without a poster, which thumbnails and the canvas show in its place";
}

impl fmt::Display for ProblemCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One lint finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub code: ProblemCode,
    pub severity: ProblemSeverity,
    pub message: String,
    /// The file the problem belongs to: the compiler fails the container that file holds.
    pub file: String,
}

impl Problem {
    /// The problem `code` in `file`, its template filled from `data` and `file`. A placeholder without a value stays
    /// as written.
    pub fn new(code: ProblemCode, file: &str, data: &[(&str, &str)]) -> Problem {
        Problem {
            code,
            severity: code.severity(),
            message: fill(code.template(), file, data),
            file: file.to_owned(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == ProblemSeverity::Error
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Replaces each `{key}` (`[a-z]+`) with its value.
fn fill(template: &str, file: &str, data: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from_brace) = rest.split_at(open);
        out.push_str(before);
        let after = from_brace.get(1..).unwrap_or("");
        let key_len = after.bytes().take_while(u8::is_ascii_lowercase).count();
        let (key, tail) = after.split_at(key_len);
        let value = data
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .or_else(|| (key == "file").then_some(file));
        match (key_len > 0, tail.strip_prefix('}'), value) {
            (true, Some(tail), Some(value)) => {
                out.push_str(value);
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_templates_from_the_file_and_data() {
        let problem = Problem::new(
            ProblemCode::TooManySlides,
            "presentation.json",
            &[("count", "301"), ("max", "300")],
        );
        assert_eq!(
            problem.message,
            "The presentation has 301 slides; at most 300 are supported"
        );
        assert_eq!(problem.severity, ProblemSeverity::Error);
        let unfilled = Problem::new(ProblemCode::LayoutMissing, "slides/a.html", &[]);
        assert_eq!(
            unfilled.message,
            "slides/a.html names the layout {layout}, which has no layout file"
        );
        assert_eq!(fill("{} {A} {x", "f", &[]), "{} {A} {x");
    }

    #[test]
    fn keeps_one_registry_of_codes_names_and_severities() {
        assert!(ProblemCode::ALL.len() > 20);
        for code in ProblemCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.name());
            let warning = matches!(
                code,
                ProblemCode::OrphanFile | ProblemCode::UnlistedDocument
            );
            assert_eq!(
                code.severity() == ProblemSeverity::Warning,
                warning,
                "{code}"
            );
        }
    }
}
