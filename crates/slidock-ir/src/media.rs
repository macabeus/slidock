//! Media entries and plans: each media item described once, keyed by the SHA-256 of its bytes as stored in the source,
//! and what the import does with it.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::Sha256;
use crate::origin::Origin;
use crate::units::Ms;
use crate::validate::{Validate, Validator};

/// A media type: `^[a-z]+/[a-z0-9.+-]{1,64}$`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mime(String);

impl Mime {
    pub fn new(text: impl Into<String>) -> Result<Mime, InvalidMime> {
        let text = text.into();
        let valid = text.split_once('/').is_some_and(|(top, sub)| {
            !top.is_empty()
                && top.bytes().all(|b| b.is_ascii_lowercase())
                && (1..=64).contains(&sub.len())
                && sub.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-')
                })
        });
        if valid {
            Ok(Mime(text))
        } else {
            Err(InvalidMime(text))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a media type: {0:?}")]
pub struct InvalidMime(pub String);

impl fmt::Display for Mime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Mime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Mime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Mime::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// An output file: `^media/[0-9a-f]{64}\.[a-z0-9]{1,8}$`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaFile(String);

impl MediaFile {
    pub fn new(text: impl Into<String>) -> Result<MediaFile, InvalidMediaFile> {
        let text = text.into();
        let valid = text
            .strip_prefix("media/")
            .and_then(|rest| rest.split_once('.'))
            .is_some_and(|(sha, ext)| {
                Sha256::parse(sha).is_ok()
                    && (1..=8).contains(&ext.len())
                    && ext
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if valid {
            Ok(MediaFile(text))
        } else {
            Err(InvalidMediaFile(text))
        }
    }

    /// `media/<sha256>.<ext>`.
    pub fn of(sha256: &Sha256, ext: &str) -> Result<MediaFile, InvalidMediaFile> {
        MediaFile::new(format!("media/{sha256}.{ext}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a media file path: {0:?}")]
pub struct InvalidMediaFile(pub String);

impl Serialize for MediaFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for MediaFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        MediaFile::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanAction {
    Copy,
    Transcode,
    Omit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanOutput {
    #[serde(rename = "type")]
    pub mime: Mime,
    pub file: MediaFile,
}

/// What the import does with a media item; the job does the I/O.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaPlan {
    pub action: PlanAction,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub output: Option<PlanOutput>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub reason: Option<String>,
}

impl Validate for MediaPlan {
    fn validate(&self, v: &mut Validator) {
        if let Some(reason) = &self.reason {
            v.at("reason", |v| v.string(reason, 0, 256));
        }
    }
}

macro_rules! media_entry {
    ($(#[$meta:meta])* $name:ident, plan: [$(#[$plan_meta:meta])*] $plan:ty) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        pub struct $name {
            pub sha256: Sha256,
            /// Sniffed from the bytes rather than the package's claims.
            #[serde(rename = "type")]
            pub mime: Mime,
            pub bytes: u64,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub width: Option<u64>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub height: Option<u64>,
            /// More than one for an animated image.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub frames: Option<u64>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub codec: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub duration_ms: Option<Ms>,
            pub origin: Origin,
            $(#[$plan_meta])*
            pub plan: $plan,
        }

        impl Validate for $name {
            fn validate(&self, v: &mut Validator) {
                if let Some(codec) = &self.codec {
                    v.at("codec", |v| v.string(codec, 0, 32));
                }
                v.field("durationMs", &self.duration_ms);
                v.field("origin", &self.origin);
                v.field("plan", &self.plan);
            }
        }
    };
}

media_entry!(
    /// A media item's description; `plan` comes from plan-media.
    MediaEntry,
    plan: [#[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]] Option<MediaPlan>
);
media_entry!(
    /// A planned media item.
    ResolvedMediaEntry,
    plan: [] MediaPlan
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_types_and_files() {
        assert!(Mime::new("image/svg+xml").is_ok());
        assert!(Mime::new("video/quicktime").is_ok());
        assert!(Mime::new("Image/png").is_err());
        assert!(Mime::new("image/").is_err());
        let sha = Sha256::of(b"x");
        assert!(MediaFile::of(&sha, "png").is_ok());
        assert!(MediaFile::of(&sha, "PNG").is_err());
        assert!(MediaFile::new("media/abc.png").is_err());
        assert!(MediaFile::new(format!("fonts/{sha}.woff2")).is_err());
    }
}
