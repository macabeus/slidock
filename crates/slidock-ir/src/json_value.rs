//! Raw source values: `ext` bags and Foreign payloads, values outside the IR's model that every pass preserves.

use std::fmt;

use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::de::coded;
use crate::diagnostics::codes;
use crate::units::Check;
use crate::validate::{Validate, Validator};

/// Raw numbers are finite, |x| < 1e15 and on a 1e-6 grid, so they print as plain decimals.
pub const JSON_NUMBER_LIMIT: f64 = 1e15;
const JSON_DIGITS: u8 = 6;

/// A raw source value.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    /// Members in JS iteration order (`de::js_order`).
    Object(IndexMap<String, JsonValue>),
}

/// `Number(x.toFixed(6))`.
fn on_grid(x: f64) -> f64 {
    slidock_canon::to_fixed(x, JSON_DIGITS)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(f64::NAN)
}

impl JsonValue {
    /// A number normalised at the frontend boundary (`toJsonValue`): non-finite becomes `null`, |x| ≥ 1e15 its
    /// JS string, anything else is rounded to the 1e-6 grid, `-0` to `0`.
    pub fn number(x: f64) -> JsonValue {
        if !x.is_finite() {
            return JsonValue::Null;
        }
        if x.abs() >= JSON_NUMBER_LIMIT {
            return JsonValue::String(slidock_canon::to_js_string(x));
        }
        let rounded = on_grid(x);
        JsonValue::Number(if rounded == 0.0 { 0.0 } else { rounded })
    }

    pub fn string(text: impl Into<String>) -> JsonValue {
        JsonValue::String(text.into())
    }
}

impl From<bool> for JsonValue {
    fn from(value: bool) -> Self {
        JsonValue::Bool(value)
    }
}

impl From<&str> for JsonValue {
    fn from(value: &str) -> Self {
        JsonValue::String(value.to_owned())
    }
}

impl From<String> for JsonValue {
    fn from(value: String) -> Self {
        JsonValue::String(value)
    }
}

impl From<f64> for JsonValue {
    fn from(value: f64) -> Self {
        JsonValue::number(value)
    }
}

impl Serialize for JsonValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            JsonValue::Null => serializer.serialize_unit(),
            JsonValue::Bool(b) => serializer.serialize_bool(*b),
            JsonValue::Number(n) => serializer.serialize_f64(*n),
            JsonValue::String(s) => serializer.serialize_str(s),
            JsonValue::Array(items) => items.serialize(serializer),
            JsonValue::Object(members) => members.serialize(serializer),
        }
    }
}

struct JsonValueVisitor;

impl JsonValueVisitor {
    fn number<E: de::Error>(x: f64) -> Result<JsonValue, E> {
        if on_grid(x) != x {
            return Err(coded(codes::IR0007, "Not a multiple of 1e-6"));
        }
        Ok(JsonValue::Number(x))
    }
}

impl<'de> Visitor<'de> for JsonValueVisitor {
    type Value = JsonValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<JsonValue, E> {
        Ok(JsonValue::Bool(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<JsonValue, E> {
        Self::number(v)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<JsonValue, E> {
        Self::number(v as f64)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<JsonValue, E> {
        Self::number(v as f64)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<JsonValue, E> {
        Ok(JsonValue::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<JsonValue, E> {
        Ok(JsonValue::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(JsonValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
        let mut members = IndexMap::new();
        while let Some((key, value)) = map.next_entry::<String, JsonValue>()? {
            members.insert(key, value);
        }
        crate::de::js_order(&mut members);
        Ok(JsonValue::Object(members))
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonValueVisitor)
    }
}

impl Validate for JsonValue {
    fn validate(&self, v: &mut Validator) {
        match self {
            JsonValue::Number(x) => {
                v.checks(
                    *x,
                    &[
                        Check::Min {
                            limit: -JSON_NUMBER_LIMIT,
                            exclusive: true,
                        },
                        Check::Max {
                            limit: JSON_NUMBER_LIMIT,
                            exclusive: true,
                        },
                    ],
                );
            }
            JsonValue::String(s) => v.string(s, 0, 65_536),
            JsonValue::Array(items) => items.validate(v),
            JsonValue::Object(members) => members.validate(v),
            JsonValue::Null | JsonValue::Bool(_) => {}
        }
    }
}

crate::de::partial_record! {
    keys: crate::formats::FileFormat::ALL.map(crate::formats::FileFormat::as_str);
    /// Format-namespaced values outside the IR's model, keyed by the format that wrote them.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Ext {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional_js_record"
        )]
        pub pptx: Option<IndexMap<String, JsonValue>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional_js_record"
        )]
        pub odp: Option<IndexMap<String, JsonValue>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional_js_record"
        )]
        pub pdf: Option<IndexMap<String, JsonValue>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::de::optional_js_record"
        )]
        pub keynote: Option<IndexMap<String, JsonValue>>,
    }
}

impl Validate for Ext {
    fn validate(&self, v: &mut Validator) {
        v.field("pptx", &self.pptx);
        v.field("odp", &self.odp);
        v.field("pdf", &self.pdf);
        v.field("keynote", &self.keynote);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_numbers_at_the_boundary() {
        assert_eq!(JsonValue::number(1.0 / 3.0), JsonValue::Number(0.333333));
        assert_eq!(JsonValue::number(f64::NAN), JsonValue::Null);
        assert_eq!(
            JsonValue::number(2e20),
            JsonValue::String("200000000000000000000".into())
        );
        assert_eq!(JsonValue::number(-0.0), JsonValue::Number(0.0));
        assert_eq!(
            crate::canonical::to_json(&JsonValue::number(-0.0)).unwrap(),
            "0"
        );
    }

    #[test]
    fn accepts_grid_numbers_and_bounds_the_rest() {
        let value: JsonValue =
            serde_json::from_str(r#"{"x":[1.5,"a",true,null,{"y":0.142857}]}"#).unwrap();
        assert!(crate::validate::validate(&value).is_empty());
        let big: JsonValue = serde_json::from_str("1e15").unwrap();
        assert!(!crate::validate::validate(&big).is_empty());
        assert!(serde_json::from_str::<JsonValue>(r#"{"x":0.3333333}"#).is_err());
    }
}
