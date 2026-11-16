//! Values that can hold only one thing: zod's `z.literal(...)`.

use serde::de::{self, Deserializer, Unexpected};
use serde::{Deserialize, Serialize, Serializer};
use ts_rs::{Config, TS};

/// `true` and nothing else (zod's `z.literal(true)`): a flag that is present or absent, never `false`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct True;

impl Serialize for True {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for True {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match bool::deserialize(deserializer)? {
            true => Ok(True),
            false => Err(de::Error::invalid_value(Unexpected::Bool(false), &"true")),
        }
    }
}

impl TS for True {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &Config) -> String {
        "true".to_owned()
    }
    fn inline(cfg: &Config) -> String {
        Self::name(cfg)
    }
}

/// The number `1`: the corpus manifest's `version`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct One;

impl Serialize for One {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for One {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let n = f64::deserialize(deserializer)?;
        if n == 1.0 {
            Ok(One)
        } else {
            Err(de::Error::invalid_value(Unexpected::Float(n), &"1"))
        }
    }
}

impl TS for One {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &Config) -> String {
        "1".to_owned()
    }
    fn inline(cfg: &Config) -> String {
        Self::name(cfg)
    }
}

/// A string that can be one value only (a schema name, `"srgb"`): a one-variant enum, so serde refuses any other
/// string and `ts-rs` prints the literal type.
macro_rules! literal {
    ($(#[$doc:meta])* $name:ident = $wire:literal) => {
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, ts_rs::TS,
        )]
        pub enum $name {
            #[default]
            #[serde(rename = $wire)]
            Value,
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                $wire
            }
        }
    };
}
pub(crate) use literal;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_accepts_only_true() {
        assert_eq!(serde_json::to_string(&True).unwrap(), "true");
        assert_eq!(serde_json::from_str::<True>("true").unwrap(), True);
        assert!(serde_json::from_str::<True>("false").is_err());
        assert!(serde_json::from_str::<True>("1").is_err());
    }

    #[test]
    fn one_accepts_only_one() {
        assert_eq!(serde_json::to_string(&One).unwrap(), "1");
        assert_eq!(serde_json::from_str::<One>("1").unwrap(), One);
        assert!(serde_json::from_str::<One>("2").is_err());
        assert!(serde_json::from_str::<One>("\"1\"").is_err());
    }

    literal!(Sample = "sample@1");

    #[test]
    fn literal_accepts_only_its_string() {
        assert_eq!(
            serde_json::to_string(&Sample::Value).unwrap(),
            "\"sample@1\""
        );
        assert!(serde_json::from_str::<Sample>("\"sample@1\"").is_ok());
        assert!(serde_json::from_str::<Sample>("\"sample@2\"").is_err());
        assert_eq!(Sample::Value.as_str(), "sample@1");
    }
}
