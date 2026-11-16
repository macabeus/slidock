//! Where a node came from. Required on every node in both phases; passes copy it unchanged.

use crate::de::tagged_union;
use crate::diagnostics::codes;
use crate::validate::{Validate, Validator};

tagged_union! {
    /// The origin of a node.
    pub enum Origin ("kind") {
        /// A part of the source package (`ppt/slides/slide3.xml`) or a page (`page:3`).
        Source = "source" {
            part: String,
            /// XPath-like with 1-based positions: `/p:sld/p:cSld/p:spTree/p:sp[4]`.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            path: Option<String>,
            /// The native id: `163`, `5213335`.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            object: Option<String>,
        },
        /// Every origin a node was merged from, e.g. a PDF line from its text items.
        Fused = "fused" { origins: Vec<Origin> },
        /// A node a pass created.
        Synth = "synth" {
            pass: String,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            from: Option<String>,
        },
        /// A node an agent authored.
        Agent = "agent" {},
    }
}

impl Origin {
    pub fn source(part: impl Into<String>) -> Origin {
        Origin::Source {
            part: part.into(),
            path: None,
            object: None,
        }
    }

    pub fn synth(pass: impl Into<String>) -> Origin {
        Origin::Synth {
            pass: pass.into(),
            from: None,
        }
    }
}

/// A part is a path inside the package or a page, never a filesystem path: not empty, no leading `/`, `\` or `~`,
/// no drive letter, no `file:` scheme, no `..` segment.
pub fn is_package_part(part: &str) -> bool {
    let bytes = part.as_bytes();
    let drive = matches!(bytes, [letter, b':', b'/' | b'\\', ..] if letter.is_ascii_alphabetic());
    let file_scheme = bytes.len() >= 5
        && bytes
            .iter()
            .take(5)
            .copied()
            .map(|b| b.to_ascii_lowercase())
            .eq(*b"file:");
    !part.is_empty()
        && !matches!(bytes.first(), Some(b'/' | b'\\' | b'~'))
        && !drive
        && !file_scheme
        && !part.split(['/', '\\']).any(|segment| segment == "..")
}

impl Validate for Origin {
    fn validate(&self, v: &mut Validator) {
        match self {
            Origin::Source { part, path, object } => {
                v.at("part", |v| {
                    v.string(part, 0, 1024);
                    if !is_package_part(part) {
                        v.invalid(
                            codes::IR0011,
                            "An origin part is a path inside the source file",
                        );
                    }
                });
                if let Some(path) = path {
                    v.at("path", |v| v.string(path, 0, 512));
                }
                if let Some(object) = object {
                    v.at("object", |v| v.string(object, 0, 256));
                }
            }
            Origin::Fused { origins } => v.at("origins", |v| {
                v.array(origins.len(), 1, 16);
                origins.validate(v);
            }),
            Origin::Synth { pass, from } => {
                v.at("pass", |v| v.string(pass, 1, 64));
                if let Some(from) = from {
                    v.at("from", |v| v.string(from, 0, 256));
                }
            }
            Origin::Agent {} => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    fn parse(json: serde_json::Value) -> Origin {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn accepts_every_kind() {
        for json in [
            serde_json::json!({ "kind": "source", "part": "ppt/slides/slide3.xml", "path": "/p:sld/p:cSld/p:spTree/p:sp[4]", "object": "163" }),
            serde_json::json!({ "kind": "source", "part": "page:3" }),
            serde_json::json!({ "kind": "fused", "origins": [{ "kind": "source", "part": "page:3" }, { "kind": "synth", "pass": "a11y" }] }),
            serde_json::json!({ "kind": "synth", "pass": "lower-geometry", "from": "pp-256.4" }),
            serde_json::json!({ "kind": "agent" }),
        ] {
            let origin = parse(json.clone());
            assert!(validate(&origin).is_empty());
            assert_eq!(serde_json::to_value(&origin).unwrap(), json);
        }
    }

    #[test]
    fn refuses_filesystem_paths_with_ir0011() {
        for part in [
            "/Users/me/presentation.pptx",
            "C:\\presentation\\a.xml",
            "~/a",
            "file:///a",
            "FILE:a",
            "ppt/../../etc",
            "",
        ] {
            assert!(!is_package_part(part), "{part}");
            let issues = validate(&Origin::source(part));
            assert_eq!(
                issues.iter().map(|i| i.code).collect::<Vec<_>>(),
                [codes::IR0011],
                "{part}"
            );
        }
        assert!(is_package_part("ppt/a..b/c.xml"));
    }

    #[test]
    fn bounds_fused_origins() {
        let leaf = Origin::Agent {};
        let too_many = Origin::Fused {
            origins: vec![leaf.clone(); 17],
        };
        assert_eq!(validate(&too_many)[0].code, codes::IR0008);
        let none = Origin::Fused { origins: vec![] };
        assert_eq!(validate(&none)[0].code, codes::IR0001);
    }
}
