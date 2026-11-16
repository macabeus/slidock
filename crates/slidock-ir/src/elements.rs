//! Element fields both phases share: placeholders, Foreign types, crops, media kinds and trims, and the macros that
//! declare each phase's element structs and enum. The phase-specific elements are [`crate::source::Element`] and
//! [`crate::resolved::Element`].

use serde::{Deserialize, Serialize};

use crate::ids::NodeId;
use crate::units::{Ms, Ratio};
use crate::validate::{Validate, Validator};

/// OOXML's placeholder types, which every format's roles map onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaceholderRole {
    Title,
    CtrTitle,
    SubTitle,
    Body,
    Obj,
    Pic,
    Chart,
    Tbl,
    Media,
    Dt,
    Ftr,
    SldNum,
    Hdr,
    Notes,
    Other,
}

impl PlaceholderRole {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            PlaceholderRole::Title => "title",
            PlaceholderRole::CtrTitle => "ctrTitle",
            PlaceholderRole::SubTitle => "subTitle",
            PlaceholderRole::Body => "body",
            PlaceholderRole::Obj => "obj",
            PlaceholderRole::Pic => "pic",
            PlaceholderRole::Chart => "chart",
            PlaceholderRole::Tbl => "tbl",
            PlaceholderRole::Media => "media",
            PlaceholderRole::Dt => "dt",
            PlaceholderRole::Ftr => "ftr",
            PlaceholderRole::SldNum => "sldNum",
            PlaceholderRole::Hdr => "hdr",
            PlaceholderRole::Notes => "notes",
            PlaceholderRole::Other => "other",
        }
    }
}

/// An element's placeholder role. `target` is an explicit binding the source records (Keynote); bind-placeholders
/// matches the others.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub role: PlaceholderRole,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub index: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub target: Option<NodeId>,
}

impl Validate for Placeholder {
    fn validate(&self, _: &mut Validator) {}
}

/// What a Foreign node stands for: content the IR does not model, or a table the frontend could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ForeignType {
    Table,
    Chart,
    Smartart,
    Ole,
    Math,
    Ink,
    #[serde(rename = "3d")]
    ThreeD,
    Zoom,
    Unknown,
}

impl ForeignType {
    /// The name the IR's JSON spells.
    pub const fn as_str(self) -> &'static str {
        match self {
            ForeignType::Table => "table",
            ForeignType::Chart => "chart",
            ForeignType::Smartart => "smartart",
            ForeignType::Ole => "ole",
            ForeignType::Math => "math",
            ForeignType::Ink => "ink",
            ForeignType::ThreeD => "3d",
            ForeignType::Zoom => "zoom",
            ForeignType::Unknown => "unknown",
        }
    }
}

/// Crops as fractions of the image per side; negative values pad.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub l: Ratio,
    pub t: Ratio,
    pub r: Ratio,
    pub b: Ratio,
}

impl Crop {
    /// Whether any side crops or pads.
    pub fn is_set(&self) -> bool {
        [self.l, self.t, self.r, self.b]
            .iter()
            .any(|side| side.steps() != 0)
    }
}

impl Validate for Crop {
    fn validate(&self, v: &mut Validator) {
        v.field("l", &self.l);
        v.field("t", &self.t);
        v.field("r", &self.r);
        v.field("b", &self.b);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Trim {
    pub start_ms: Ms,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub end_ms: Option<Ms>,
}

impl Validate for Trim {
    fn validate(&self, v: &mut Validator) {
        v.field("startMs", &self.start_ms);
        v.field("endMs", &self.end_ms);
    }
}

/// Serde's `skip_serializing_if` for a flag the IR writes only when it is set.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

/// What a Foreign node prints in its place, when the source has one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(serialize = "E: Serialize", deserialize = "E: Deserialize<'de>")
)]
pub struct ForeignFallback<E> {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub media: Option<crate::ids::Sha256>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub elements: Option<Vec<E>>,
}

impl<E: Validate> Validate for ForeignFallback<E> {
    fn validate(&self, v: &mut Validator) {
        v.field("elements", &self.elements);
    }
}

/// The longest match key, in UTF-16 code units.
pub const MAX_MATCH_KEY: usize = 256;

/// Declares an element struct: the fields every element has (in zod's `commonShape` order), the phase's `frame`,
/// then its own fields.
macro_rules! element_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            frame: [$(#[$frame_meta:meta])*] $frame:ty;
            $( $(#[$fmeta:meta])* pub $field:ident : $fty:ty, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        pub struct $name {
            pub id: $crate::ids::ElementId,
            pub origin: $crate::origin::Origin,
            /// The source's own object name (`Google Shape;163;p35`), informational.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub name: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub placeholder: Option<$crate::elements::Placeholder>,
            /// The node this one inherits unset values from: provenance once resolved.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub inherits: Option<$crate::ids::ElementId>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub hidden: Option<bool>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub opacity: Option<$crate::units::UnitRatio>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub link: Option<$crate::text::Link>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub alt: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub ext: Option<$crate::json_value::Ext>,
            /// The Magic Move / Morph key that pairs this element with one on a neighbouring slide (slidock): the
            /// player morphs the elements of two slides that share a key.
            #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
            pub match_key: Option<String>,
            $(#[$frame_meta])*
            pub frame: $frame,
            $( $(#[$fmeta])* pub $field: $fty, )*
        }

        impl $name {
            /// Validates the common fields and the frame.
            fn validate_common(&self, v: &mut $crate::validate::Validator) {
                v.field("origin", &self.origin);
                if let Some(name) = &self.name {
                    v.at("name", |v| v.string(name, 0, 256));
                }
                v.field("opacity", &self.opacity);
                v.field("link", &self.link);
                if let Some(alt) = &self.alt {
                    v.at("alt", |v| v.string(alt, 0, $crate::text::MAX_STRING));
                }
                v.field("ext", &self.ext);
                if let Some(key) = &self.match_key {
                    v.at("matchKey", |v| v.string(key, 1, $crate::elements::MAX_MATCH_KEY));
                }
                v.field("frame", &self.frame);
            }
        }
    };
}
pub(crate) use element_struct;

/// Declares a phase's element enum, tagged by `kind`, with accessors for the common fields. Each variant is boxed,
/// so an element is two words wherever it is built, matched or moved: a debug build compiles a deck on a 1 MiB
/// stack (a wasm worker's).
macro_rules! element_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident, frame: $frame:ty {
            $( $variant:ident($ty:ty) = $wire:literal ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, serde::Serialize)]
        #[serde(tag = "kind")]
        pub enum $name {
            $( #[serde(rename = $wire)] $variant(Box<$ty>), )*
        }

        $(
            impl From<$ty> for $name {
                fn from(element: $ty) -> Self {
                    $name::$variant(Box::new(element))
                }
            }
        )*

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let (wire, payload) = $crate::de::split_tag(deserializer, "kind")?;
                match wire.as_str() {
                    $( $wire => $crate::de::from_value::<Box<$ty>, _>(payload).map($name::$variant), )*
                    other => Err($crate::de::unknown_tag(other, "kind", &[$($wire),*])),
                }
            }
        }

        impl $name {
            /// The JSON tag (`shape`, `picture`, …).
            pub fn kind(&self) -> &'static str {
                match self {
                    $( $name::$variant(_) => $wire, )*
                }
            }

            pub fn id(&self) -> &$crate::ids::ElementId {
                match self {
                    $( $name::$variant(e) => &e.id, )*
                }
            }

            pub fn origin(&self) -> &$crate::origin::Origin {
                match self {
                    $( $name::$variant(e) => &e.origin, )*
                }
            }

            pub fn name(&self) -> Option<&str> {
                match self {
                    $( $name::$variant(e) => e.name.as_deref(), )*
                }
            }

            pub fn placeholder(&self) -> Option<&$crate::elements::Placeholder> {
                match self {
                    $( $name::$variant(e) => e.placeholder.as_ref(), )*
                }
            }

            pub fn inherits(&self) -> Option<&$crate::ids::ElementId> {
                match self {
                    $( $name::$variant(e) => e.inherits.as_ref(), )*
                }
            }

            pub fn hidden(&self) -> Option<bool> {
                match self {
                    $( $name::$variant(e) => e.hidden, )*
                }
            }

            pub fn opacity(&self) -> Option<$crate::units::UnitRatio> {
                match self {
                    $( $name::$variant(e) => e.opacity, )*
                }
            }

            pub fn link(&self) -> Option<&$crate::text::Link> {
                match self {
                    $( $name::$variant(e) => e.link.as_ref(), )*
                }
            }

            pub fn alt(&self) -> Option<&str> {
                match self {
                    $( $name::$variant(e) => e.alt.as_deref(), )*
                }
            }

            pub fn ext(&self) -> Option<&$crate::json_value::Ext> {
                match self {
                    $( $name::$variant(e) => e.ext.as_ref(), )*
                }
            }

            pub fn match_key(&self) -> Option<&str> {
                match self {
                    $( $name::$variant(e) => e.match_key.as_deref(), )*
                }
            }

            pub fn match_key_mut(&mut self) -> &mut Option<String> {
                match self {
                    $( $name::$variant(e) => &mut e.match_key, )*
                }
            }

            pub fn frame(&self) -> &$frame {
                match self {
                    $( $name::$variant(e) => &e.frame, )*
                }
            }

            pub fn frame_mut(&mut self) -> &mut $frame {
                match self {
                    $( $name::$variant(e) => &mut e.frame, )*
                }
            }
        }

        impl $crate::validate::Validate for $name {
            fn validate(&self, v: &mut $crate::validate::Validator) {
                v.element(self.id(), |v| match self {
                    $( $name::$variant(e) => e.validate(v), )*
                });
            }
        }
    };
}
pub(crate) use element_enum;
