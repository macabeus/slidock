//! Presentation parts both phases share: themes, text styles, the failed-container marker, the slide size, the defaults
//! and the phase.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::color::SchemeColors;
use crate::de::literal_tag;
use crate::diagnostics::{DiagnosticCode, is_code_shaped};
use crate::ids::ThemeId;
use crate::json_value::Ext;
use crate::origin::Origin;
use crate::text::{Autofit, BodyProps, DefaultTextStyles, LevelStyles};
use crate::units::Size;
use crate::validate::{Validate, Validator};

literal_tag!(
    /// `$schema` of every canonical dump, which is also how the IR frontend recognises one. The IR frontend refuses a
    /// document naming another `$schema`, or none.
    SchemaUrl = "urn:slidock:presentation-ir:1"
);

/// The code a failed container records: any diagnostic code shape (`^[A-Z]+\d{4}$` over the known prefixes).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FailedCode(String);

impl FailedCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The registered code, if it is one.
    pub fn code(&self) -> Option<DiagnosticCode> {
        DiagnosticCode::lookup(&self.0)
    }
}

impl From<DiagnosticCode> for FailedCode {
    fn from(code: DiagnosticCode) -> Self {
        FailedCode(code.code().to_owned())
    }
}

impl Serialize for FailedCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FailedCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        if is_code_shaped(&code) {
            Ok(FailedCode(code))
        } else {
            Err(serde::de::Error::custom(format_args!(
                "Invalid string: {code:?} is not a diagnostic code"
            )))
        }
    }
}

/// A container that failed to compile: it keeps its id, place, links and background, and an empty element list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failed {
    pub code: FailedCode,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub pass: Option<String>,
}

impl Failed {
    pub fn new(code: DiagnosticCode, pass: Option<String>) -> Self {
        Failed {
            code: code.into(),
            pass,
        }
    }
}

impl Validate for Failed {
    fn validate(&self, v: &mut Validator) {
        if let Some(pass) = &self.pass {
            v.at("pass", |v| v.string(pass, 0, 64));
        }
    }
}

/// A theme's fonts for one role: the Latin face, and East Asian and complex-script ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFonts {
    pub latin: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ea: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub cs: Option<String>,
}

impl Validate for ThemeFonts {
    fn validate(&self, v: &mut Validator) {
        v.at("latin", |v| v.string(&self.latin, 0, 256));
        if let Some(ea) = &self.ea {
            v.at("ea", |v| v.string(ea, 0, 256));
        }
        if let Some(cs) = &self.cs {
            v.at("cs", |v| v.string(cs, 0, 256));
        }
    }
}

/// The heading (`major`) and body (`minor`) fonts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MajorMinor {
    pub major: ThemeFonts,
    pub minor: ThemeFonts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub id: ThemeId,
    pub origin: Origin,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub name: Option<String>,
    pub colors: SchemeColors,
    pub fonts: MajorMinor,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub ext: Option<Ext>,
}

impl Validate for Theme {
    fn validate(&self, v: &mut Validator) {
        v.field("origin", &self.origin);
        if let Some(name) = &self.name {
            v.at("name", |v| v.string(name, 0, 256));
        }
        v.field("colors", &self.colors);
        v.at("fonts", |v| {
            v.field("major", &self.fonts.major);
            v.field("minor", &self.fonts.minor);
        });
        v.field("ext", &self.ext);
    }
}

/// A master's text styles per placeholder class: title, body, and other (date, footer, slide number).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyles {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub title: Option<LevelStyles>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub body: Option<LevelStyles>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub other: Option<LevelStyles>,
}

impl Validate for TextStyles {
    fn validate(&self, v: &mut Validator) {
        v.field("title", &self.title);
        v.field("body", &self.body);
        v.field("other", &self.other);
    }
}

/// The slide size in points; both sides positive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideSize {
    pub w: Size,
    pub h: Size,
}

impl Validate for SlideSize {
    fn validate(&self, v: &mut Validator) {
        for (key, side) in [("w", &self.w), ("h", &self.h)] {
            v.at(key, |v| {
                side.validate(v);
                v.min(side.to_f64(), 0.0, true);
            });
        }
    }
}

/// How a format states its defaults, so passes read them whatever the format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub text: Box<DefaultTextStyles>,
    pub body: BodyProps<Autofit>,
}

impl Validate for Defaults {
    fn validate(&self, v: &mut Validator) {
        v.field("text", &self.text);
        v.field("body", &self.body);
    }
}

/// Which phase a presentation is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Source,
    Resolved,
}

impl Phase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Source => "source",
            Phase::Resolved => "resolved",
        }
    }
}

literal_tag!(
    /// The `phase` of a resolved presentation.
    ResolvedPhase = "resolved"
);

/// Presentation limits the verifier enforces.
pub mod limits {
    pub const SLIDES: usize = 300;
    pub const ELEMENTS_PER_CONTAINER: usize = 5_000;
    pub const GROUP_DEPTH: usize = 16;
    pub const INHERITS_CHAIN: usize = 8;
    /// UTF-16 code units of run and field text per body.
    pub const TEXT_PER_BODY: usize = 1_000_000;
    /// Canonical JSON bytes.
    pub const EXT_PER_NODE: usize = 16 * 1024;
    pub const EXT_PER_PRESENTATION: usize = 1024 * 1024;
    pub const PAYLOAD: usize = 256 * 1024;
    /// The grid slots of all a container's tables, which each draw a box and a text body.
    pub const CELLS_PER_CONTAINER: usize = 10_000;
    /// The grid slots of all the presentation's tables, which bound the cells a small source can repeat into being.
    pub const CELLS_PER_PRESENTATION: usize = 50_000;
}

#[cfg(test)]
mod tests {
    use super::SchemaUrl;

    #[test]
    fn reads_only_its_own_schema() {
        let read = |text: &str| serde_json::from_value::<SchemaUrl>(serde_json::Value::from(text));
        assert_eq!(SchemaUrl::WIRE, "urn:slidock:presentation-ir:1");
        assert!(read("urn:slidock:presentation-ir:1").is_ok());
        for other in [
            "urn:slidock:presentation-ir:2",
            "https://example.com/presentation-ir.schema.json",
            "",
        ] {
            assert!(read(other).is_err(), "{other:?}");
        }
        assert_eq!(
            serde_json::to_value(SchemaUrl).unwrap(),
            "urn:slidock:presentation-ir:1"
        );
    }
}
