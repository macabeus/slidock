//! Native effect ids (provenance) and the native effect a step or transition carries: the id the source wrote and
//! the vocabulary entry it maps to.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use super::build::BuildEffect;
use super::transition::TransitionEffect;
use crate::formats::{EFFECT_NAME_MAX, EffectId, SourceFormat};
use crate::validate::{Validate, Validator};

/// Where an effect came from, as the source names it: `pptx:push`, `pptx:entr.2/4` (class, preset id, subtype),
/// `odp:ooo-entrance-fly-in`, `odp:pushWipe/fromLeft`, `keynote:apple:move in`. The name is printable ASCII,
/// spaces included but not at either end, 1 to 120 characters. Provenance only: a player decodes the vocabulary
/// entry, never this.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[ts(export_to = "ir/", type = "string")]
pub struct NativeId {
    format: SourceFormat,
    name: String,
}

/// A string that is not `<format>:<name>` with a name as [`NativeId`] describes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a native effect id: {0:?}")]
pub struct InvalidNativeId(pub String);

fn name_char(c: char) -> bool {
    (' '..='~').contains(&c)
}

impl NativeId {
    /// The id of a native name: characters outside printable ASCII, and spaces at either end, become `_` (one per
    /// UTF-16 code unit); the name keeps its first 120 characters.
    pub fn new(format: SourceFormat, native: &str) -> NativeId {
        let mut name = String::new();
        for c in native.chars() {
            if name_char(c) {
                name.push(c);
            } else {
                for _ in 0..c.len_utf16() {
                    name.push('_');
                }
            }
        }
        name.truncate(EFFECT_NAME_MAX);
        if name.starts_with(' ') {
            name.replace_range(..1, "_");
        }
        if name.ends_with(' ') {
            name.pop();
            name.push('_');
        }
        if name.is_empty() {
            name.push('_');
        }
        NativeId { format, name }
    }

    pub fn parse(text: &str) -> Result<NativeId, InvalidNativeId> {
        let invalid = || InvalidNativeId(text.to_owned());
        let (format, name) = text.split_once(':').ok_or_else(invalid)?;
        let format = SourceFormat::parse(format).ok_or_else(invalid)?;
        if name.is_empty()
            || name.len() > EFFECT_NAME_MAX
            || !name.chars().all(name_char)
            || name.starts_with(' ')
            || name.ends_with(' ')
        {
            return Err(invalid());
        }
        Ok(NativeId {
            format,
            name: name.to_owned(),
        })
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The IR's opaque effect id of the same name (spaces become `_`).
    pub fn effect_id(&self) -> EffectId {
        EffectId::new(self.format, &self.name)
    }
}

impl fmt::Display for NativeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.format.as_str(), self.name)
    }
}

impl Serialize for NativeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NativeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        NativeId::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A build step's native effect: the source's id and the vocabulary entry it plays as
/// (`{"id": "pptx:entr.2/4", "effect": "flyIn", "params": {...}}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct NativeStep {
    pub id: NativeId,
    #[serde(flatten)]
    pub effect: BuildEffect,
}

impl Validate for NativeStep {
    fn validate(&self, v: &mut Validator) {
        v.field("params", &self.effect);
    }
}

/// A transition's native effect: the source's id and the vocabulary entry it plays as
/// (`{"id": "pptx:push", "effect": "push", "params": {"dir": "l"}}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "ir/")]
pub struct NativeTransition {
    pub id: NativeId,
    #[serde(flatten)]
    pub effect: TransitionEffect,
}

impl Validate for NativeTransition {
    fn validate(&self, v: &mut Validator) {
        v.field("params", &self.effect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FileFormat;

    #[test]
    fn ids_keep_spaces_inside_the_name() {
        let id = NativeId::new(FileFormat::Keynote.into(), "apple:move in");
        assert_eq!(id.to_string(), "keynote:apple:move in");
        assert_eq!(NativeId::parse("keynote:apple:move in"), Ok(id.clone()));
        assert_eq!(id.effect_id().to_string(), "keynote:apple:move_in");
        assert_eq!(NativeId::parse("pptx:entr.2/4").unwrap().name(), "entr.2/4");
        for bad in [
            "pptx:",
            "pptx: fade",
            "pptx:fade ",
            "figma:fade",
            "pptx:f\u{e9}",
            "fade",
        ] {
            assert!(NativeId::parse(bad).is_err(), "{bad}");
        }
        assert!(NativeId::parse(&format!("odp:{}", "x".repeat(121))).is_err());
        assert!(NativeId::parse(&format!("odp:{}", "x".repeat(120))).is_ok());
    }

    #[test]
    fn new_makes_any_name_valid() {
        for (raw, name) in [
            (" padded ", "_padded_"),
            ("", "_"),
            ("caf\u{e9}", "caf_"),
            ("a\u{1F600}", "a__"),
        ] {
            let id = NativeId::new(SourceFormat::Ir, raw);
            assert_eq!(id.name(), name);
            assert_eq!(NativeId::parse(&id.to_string()), Ok(id));
        }
        let long = NativeId::new(SourceFormat::Ir, &format!("{} x", "y".repeat(119)));
        assert_eq!(long.name().len(), 120);
        assert!(NativeId::parse(&long.to_string()).is_ok());
    }
}
