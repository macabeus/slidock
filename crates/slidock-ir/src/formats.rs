//! Source formats and effect ids.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The formats a file can be, each owning an `ext` bag, a Foreign dialect and an effect-id namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileFormat {
    Pptx,
    Odp,
    Pdf,
    Keynote,
}

impl FileFormat {
    pub const ALL: [FileFormat; 4] = [
        FileFormat::Pptx,
        FileFormat::Odp,
        FileFormat::Pdf,
        FileFormat::Keynote,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            FileFormat::Pptx => "pptx",
            FileFormat::Odp => "odp",
            FileFormat::Pdf => "pdf",
            FileFormat::Keynote => "keynote",
        }
    }
}

/// Every input of the compiler: the file formats plus agent-written IR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceFormat {
    File(FileFormat),
    Ir,
}

impl SourceFormat {
    pub const ALL: [SourceFormat; 5] = [
        SourceFormat::File(FileFormat::Pptx),
        SourceFormat::File(FileFormat::Odp),
        SourceFormat::File(FileFormat::Pdf),
        SourceFormat::File(FileFormat::Keynote),
        SourceFormat::Ir,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            SourceFormat::File(format) => format.as_str(),
            SourceFormat::Ir => "ir",
        }
    }

    /// The id prefix of the format's nodes, so ids from two sources stay distinct in one presentation.
    pub const fn id_prefix(self) -> IdPrefix {
        match self {
            SourceFormat::File(FileFormat::Pptx) => IdPrefix::Pp,
            SourceFormat::File(FileFormat::Odp) => IdPrefix::Od,
            SourceFormat::File(FileFormat::Pdf) => IdPrefix::Pd,
            SourceFormat::File(FileFormat::Keynote) => IdPrefix::Kn,
            SourceFormat::Ir => IdPrefix::Ir,
        }
    }

    pub fn parse(text: &str) -> Option<SourceFormat> {
        Self::ALL.into_iter().find(|format| format.as_str() == text)
    }
}

impl From<FileFormat> for SourceFormat {
    fn from(format: FileFormat) -> Self {
        SourceFormat::File(format)
    }
}

impl Serialize for SourceFormat {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SourceFormat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        SourceFormat::parse(&text).ok_or_else(|| {
            serde::de::Error::unknown_variant(&text, &["pptx", "odp", "pdf", "keynote", "ir"])
        })
    }
}

/// The two-letter prefix of node ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdPrefix {
    Pp,
    Od,
    Pd,
    Kn,
    Ir,
}

impl IdPrefix {
    pub const fn as_str(self) -> &'static str {
        match self {
            IdPrefix::Pp => "pp",
            IdPrefix::Od => "od",
            IdPrefix::Pd => "pd",
            IdPrefix::Kn => "kn",
            IdPrefix::Ir => "ir",
        }
    }
}

/// The longest native effect name.
pub const EFFECT_NAME_MAX: usize = 120;

/// A timeline or transition effect: `<format>:<native name>`, the name printable ASCII (`[!-~]{1,120}`). Passes
/// and backends copy it verbatim; only the runtime interprets it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EffectId {
    format: SourceFormat,
    name: String,
}

/// A string that is not `<format>:<name>` with a printable ASCII name of 1 to 120 characters.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not an effect id: {0:?}")]
pub struct InvalidEffectId(pub String);

impl EffectId {
    /// The effect id of a native effect name; characters outside printable ASCII become `_` (one per UTF-16 code
    /// unit), and the name keeps its first 120 code units.
    pub fn new(format: SourceFormat, native: &str) -> EffectId {
        let mut name = String::new();
        for c in native.chars() {
            if ('!'..='~').contains(&c) {
                name.push(c);
            } else {
                for _ in 0..c.len_utf16() {
                    name.push('_');
                }
            }
        }
        name.truncate(EFFECT_NAME_MAX);
        if name.is_empty() {
            name.push('_');
        }
        EffectId { format, name }
    }

    pub fn parse(text: &str) -> Result<EffectId, InvalidEffectId> {
        let invalid = || InvalidEffectId(text.to_owned());
        let (format, name) = text.split_once(':').ok_or_else(invalid)?;
        let format = SourceFormat::parse(format).ok_or_else(invalid)?;
        if name.is_empty()
            || name.len() > EFFECT_NAME_MAX
            || !name.bytes().all(|b| (b'!'..=b'~').contains(&b))
        {
            return Err(invalid());
        }
        Ok(EffectId {
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
}

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.format.as_str(), self.name)
    }
}

impl Serialize for EffectId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EffectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        EffectId::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_native_names_by_format() {
        assert_eq!(
            EffectId::new(FileFormat::Pptx.into(), "fade").to_string(),
            "pptx:fade"
        );
        let magic = EffectId::new(FileFormat::Keynote.into(), "Magic Move");
        assert_eq!(magic.to_string(), "keynote:Magic_Move");
        assert_eq!(EffectId::parse(&magic.to_string()), Ok(magic));
        assert!(EffectId::parse("figma:fade").is_err());
        assert!(EffectId::parse("pptx:").is_err());
        assert!(EffectId::parse("pptx:a b").is_err());
        assert_eq!(EffectId::new(SourceFormat::Ir, "").to_string(), "ir:_");
        assert_eq!(EffectId::new(SourceFormat::Ir, "a😀").to_string(), "ir:a__");
        assert_eq!(
            EffectId::new(SourceFormat::Ir, &"x".repeat(200))
                .name()
                .len(),
            120
        );
    }

    #[test]
    fn prefixes() {
        assert_eq!(
            SourceFormat::File(FileFormat::Keynote).id_prefix().as_str(),
            "kn"
        );
        assert_eq!(SourceFormat::Ir.id_prefix().as_str(), "ir");
    }
}
