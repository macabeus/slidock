//! Deserialisation helpers shared by the IR types.
//!
//! Unions are read through a JSON value: the tag picks the variant, and the variant's payload is read with its own
//! path tracker, whose path travels inside the error message ([`decode`] takes it back out). A structural error
//! deep inside an element so keeps its full path, which the verifier needs to name the container and node, as
//! zod's issue paths do. Errors that stand for a diagnostic other than IR0001 carry its code the same way, and a
//! number's failed checks travel as a list of issues.
//!
//! Paths name what zod's do: a missing key names the key, an unknown key the object holding it, and a union's tag
//! that is missing, not a string or unknown names the tag.

use std::fmt::Display;

use indexmap::IndexMap;

use serde::de::{self, DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::diagnostics::{DiagnosticCode, codes};
use crate::parse::StructureIssue;
use crate::path::PathSeg;
use crate::validate::IssueDetail;

const MARK: char = '\u{1}';

/// An error that the verifier reports as `code` rather than IR0001.
pub(crate) fn coded<E: de::Error>(code: DiagnosticCode, reason: impl Display) -> E {
    E::custom(format_args!("{MARK}C{}{MARK}{reason}", code.code()))
}

/// An error for a number that fails `issues`, a number's checks in order ([`crate::units::failed`]).
pub(crate) fn failed_checks<E: de::Error>(issues: &[StructureIssue]) -> E {
    let wire: Vec<Value> = issues
        .iter()
        .map(|issue| match &issue.detail {
            IssueDetail::Bound { limit, .. } => {
                serde_json::json!([issue.code.code(), "limit", limit])
            }
            IssueDetail::Reason(reason) => serde_json::json!([issue.code.code(), "reason", reason]),
        })
        .collect();
    let json = serde_json::to_string(&wire).unwrap_or_default();
    let reason = issues
        .first()
        .map_or_else(String::new, |issue| match &issue.detail {
            IssueDetail::Bound { limit, .. } => format!("Out of range: {limit}"),
            IssueDetail::Reason(reason) => reason.clone(),
        });
    E::custom(format_args!("{MARK}I{json}{MARK}{reason}"))
}

/// An error at `key` below the value being read.
fn at_key<E: de::Error>(key: &str, reason: impl Display) -> E {
    let json = serde_json::to_string(&[key]).unwrap_or_default();
    E::custom(format_args!("{MARK}P{json}{MARK}{reason}"))
}

/// A structural error decoded from a deserialisation error: its path and what is wrong there.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Decoded {
    pub path: Vec<PathSeg>,
    pub issues: Vec<StructureIssue>,
}

fn decode_issues(json: &str) -> Option<Vec<StructureIssue>> {
    let wire: Vec<(String, String, String)> = serde_json::from_str(json).ok()?;
    wire.into_iter()
        .map(|(code, kind, text)| {
            let code: DiagnosticCode = code.parse().ok()?;
            let detail = if kind == "limit" {
                IssueDetail::Bound {
                    what: "number",
                    limit: text,
                }
            } else {
                IssueDetail::Reason(text)
            };
            Some(StructureIssue { code, detail })
        })
        .collect()
}

/// The field a serde `missing field`/`unknown field` message names.
fn field_named<'a>(reason: &'a str, prefix: &str) -> Option<&'a str> {
    reason.strip_prefix(prefix)?.split('`').next()
}

fn segments(path: &serde_path_to_error::Path) -> impl Iterator<Item = PathSeg> + '_ {
    path.iter().filter_map(|seg| match seg {
        serde_path_to_error::Segment::Seq { index } => Some(PathSeg::Index(*index)),
        serde_path_to_error::Segment::Map { key } => Some(PathSeg::from(key.clone())),
        serde_path_to_error::Segment::Enum { .. } | serde_path_to_error::Segment::Unknown => None,
    })
}

/// Splits a tracked error into its full path and issues.
pub(crate) fn decode(error: &serde_path_to_error::Error<serde_json::Error>) -> Decoded {
    let mut path: Vec<PathSeg> = segments(error.path()).collect();
    let message = error.inner().to_string();
    let mut rest = message.as_str();
    let mut code = None;
    let mut issues = None;
    while let Some((head, after)) = rest
        .strip_prefix(MARK)
        .and_then(|tail| tail.split_once(MARK))
    {
        if let Some(json) = head.strip_prefix('P') {
            if let Ok(inner) = serde_json::from_str::<Vec<Value>>(json) {
                path.extend(inner.into_iter().filter_map(|seg| match seg {
                    Value::String(key) => Some(PathSeg::from(key)),
                    Value::Number(n) => n.as_u64().map(|i| PathSeg::Index(i as usize)),
                    _ => None,
                }));
            }
        } else if let Some(name) = head.strip_prefix('C') {
            code = name.parse().ok();
        } else if let Some(json) = head.strip_prefix('I') {
            issues = decode_issues(json);
        }
        rest = after;
    }
    if let Some(name) = field_named(rest, "missing field `") {
        path.push(PathSeg::from(name.to_owned()));
    } else if let Some(name) = field_named(rest, "unknown field `")
        && path.last() == Some(&PathSeg::from(name.to_owned()))
    {
        path.pop();
    }
    let issues = issues.unwrap_or_else(|| {
        vec![StructureIssue {
            code: code.unwrap_or(codes::IR0001),
            detail: IssueDetail::Reason(rest.to_owned()),
        }]
    });
    Decoded { path, issues }
}

/// Re-raises a nested tracked error in the outer deserialiser, its path kept in the message.
fn nested<E: de::Error>(error: serde_path_to_error::Error<serde_json::Error>) -> E {
    let path: Vec<Value> = segments(error.path())
        .map(|seg| match seg {
            PathSeg::Key(key) => Value::String(key.into_owned()),
            PathSeg::Index(i) => Value::from(i),
        })
        .collect();
    let inner = error.inner().to_string();
    if path.is_empty() {
        E::custom(inner)
    } else {
        let json = serde_json::to_string(&path).unwrap_or_default();
        E::custom(format_args!("{MARK}P{json}{MARK}{inner}"))
    }
}

/// Reads `value` as `T` with its own path tracker.
pub(crate) fn from_value<T: DeserializeOwned, E: de::Error>(value: Value) -> Result<T, E> {
    serde_path_to_error::deserialize(value).map_err(nested)
}

/// Reads an object and removes its tag: the tag's value and the rest of the object.
pub(crate) fn split_tag<'de, D: Deserializer<'de>>(
    deserializer: D,
    tag: &'static str,
) -> Result<(String, Value), D::Error> {
    let mut map = object::<D>(Value::deserialize(deserializer)?)?;
    match map.remove(tag) {
        Some(Value::String(wire)) => Ok((wire, Value::Object(map))),
        Some(_) => Err(tag_not_string(tag)),
        None => Err(de::Error::missing_field(tag)),
    }
}

fn tag_not_string<E: de::Error>(tag: &str) -> E {
    at_key(
        tag,
        format_args!("Invalid input: the `{tag}` tag is not a string"),
    )
}

/// Reads an object and its tag, leaving the tag in place (for variants whose types carry their own tag).
pub(crate) fn peek_tag<'de, D: Deserializer<'de>>(
    deserializer: D,
    tag: &'static str,
) -> Result<(String, Map<String, Value>), D::Error> {
    let map = object::<D>(Value::deserialize(deserializer)?)?;
    match map.get(tag) {
        Some(Value::String(wire)) => Ok((wire.clone(), map)),
        Some(_) => Err(tag_not_string(tag)),
        None => Err(de::Error::missing_field(tag)),
    }
}

fn object<'de, D: Deserializer<'de>>(value: Value) -> Result<Map<String, Value>, D::Error> {
    match value {
        Value::Object(map) => Ok(map),
        other => Err(de::Error::invalid_type(unexpected(&other), &"an object")),
    }
}

pub(crate) fn unexpected(value: &Value) -> de::Unexpected<'_> {
    match value {
        Value::Null => de::Unexpected::Unit,
        Value::Bool(b) => de::Unexpected::Bool(*b),
        Value::Number(n) => de::Unexpected::Float(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => de::Unexpected::Str(s),
        Value::Array(_) => de::Unexpected::Seq,
        Value::Object(_) => de::Unexpected::Map,
    }
}

/// The error for a tag no variant has.
pub(crate) fn unknown_tag<E: de::Error>(
    wire: &str,
    tag: &str,
    expected: &'static [&'static str],
) -> E {
    at_key(
        tag,
        format_args!(
            "Invalid input: no `{tag}` {wire:?}, expected one of {}",
            expected.join(", ")
        ),
    )
}

/// Declares a union tagged by a field (`kind`, `type`) whose variants all have distinct tags, as zod's
/// `discriminatedUnion`. Each variant's fields are a strict object: unknown keys are refused.
macro_rules! tagged_union {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident ($tag:literal) {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $wire:literal { $( $(#[$fmeta:meta])* $field:ident : $fty:ty ),* $(,)? }
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, serde::Serialize)]
        #[serde(tag = $tag)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                #[serde(rename = $wire, rename_all = "camelCase")]
                $variant { $( $(#[$fmeta])* $field : $fty ),* },
            )*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let (wire, payload) = $crate::de::split_tag(deserializer, $tag)?;
                match wire.as_str() {
                    $(
                        $wire => {
                            #[derive(serde::Deserialize)]
                            #[serde(deny_unknown_fields, rename_all = "camelCase")]
                            struct Payload { $( $(#[$fmeta])* $field : $fty ),* }
                            let Payload { $($field),* } = $crate::de::from_value(payload)?;
                            Ok($name::$variant { $($field),* })
                        }
                    )*
                    other => Err($crate::de::unknown_tag(other, $tag, &[$($wire),*])),
                }
            }
        }
    };
}
pub(crate) use tagged_union;

/// Reads an object standing for zod's `partialRecord` over `keys`: a key outside them is an issue at that key
/// ("Invalid key in record"), where a strict object's unknown key is an issue at the object.
pub(crate) fn record_keys<'de, D: Deserializer<'de>>(
    deserializer: D,
    keys: &[&str],
) -> Result<Value, D::Error> {
    let value = Value::deserialize(deserializer)?;
    if let Value::Object(map) = &value
        && let Some(key) = map.keys().find(|key| !keys.contains(&key.as_str()))
    {
        return Err(at_key(key, "Invalid key in record"));
    }
    Ok(value)
}

/// Declares a struct standing for zod's `partialRecord` over `keys`, its fields the record's optional entries:
/// read through [`record_keys`], then as a strict object.
macro_rules! partial_record {
    (
        keys: $keys:expr;
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $(#[$fmeta:meta])* $fvis:vis $field:ident : $fty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(serde::Serialize)]
        $vis struct $name {
            $( $(#[$fmeta])* $fvis $field : $fty ),*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    $( $(#[$fmeta])* $field : $fty ),*
                }
                let value = $crate::de::record_keys(deserializer, &$keys)?;
                let Wire { $($field),* } = $crate::de::from_value(value)?;
                Ok($name { $($field),* })
            }
        }
    };
}
pub(crate) use partial_record;

/// Declares a zero-sized literal tag (`"rgb"`) for a struct that carries its own `kind`.
macro_rules! literal_tag {
    ($(#[$meta:meta])* $name:ident = $wire:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl $name {
            pub const WIRE: &'static str = $wire;
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($wire)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                if text == $wire {
                    Ok($name)
                } else {
                    Err(serde::de::Error::invalid_value(
                        serde::de::Unexpected::Str(&text),
                        &concat!("\"", $wire, "\""),
                    ))
                }
            }
        }
    };
}
pub(crate) use literal_tag;

/// Declares a union of structs that each carry their own literal tag (a `kind` field), as zod's
/// `discriminatedUnion` over standalone schemas. It serialises as the variant's struct.
macro_rules! self_tagged_union {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident ($tag:literal) {
            $( $(#[$vmeta:meta])* $variant:ident($ty:ty) = $wire:literal ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant($ty), )*
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    $( $name::$variant(inner) => inner.serialize(serializer), )*
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let (wire, map) = $crate::de::peek_tag(deserializer, $tag)?;
                match wire.as_str() {
                    $( $wire => $crate::de::from_value(serde_json::Value::Object(map)).map($name::$variant), )*
                    other => Err($crate::de::unknown_tag(other, $tag, &[$($wire),*])),
                }
            }
        }
    };
}
pub(crate) use self_tagged_union;

/// Serde adapter for a required field that may be `null` (zod's `T.nullable()`): unlike a plain `Option`, a
/// missing key is an error.
pub mod nullable {
    use super::*;

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        Option::<T>::deserialize(deserializer)
    }
}

/// Serde adapter for an optional field (zod's `T.optional()`), used with `default` and
/// `skip_serializing_if = "Option::is_none"`: absent is `None`, and a present key is read as `T`, so `null` is
/// refused unless `T` itself takes it (a raw JSON value). Every `Option` field of the IR goes through this adapter,
/// [`nullable`], [`nullish`] or [`double_option`].
pub mod optional {
    use super::*;

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        T::deserialize(deserializer).map(Some)
    }
}

/// Serde adapter for an optional field the source phase may also state as `null` (zod's `T.nullish()`), used with
/// `default` and `skip_serializing_if = "Option::is_none"`: absent and `null` are both `None`, written absent. The
/// resolved phase reads a source-phase [`Nullable`] this way once nothing inherits from it.
pub mod nullish {
    use super::*;

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<T>, D::Error> {
        Option::<T>::deserialize(deserializer)
    }
}

/// The array index a key stands for in a JS object: a canonical decimal below 2^32 - 1.
fn array_index(key: &str) -> Option<u32> {
    let canonical =
        key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
    key.parse::<u32>()
        .ok()
        .filter(|&index| canonical && index != u32::MAX)
}

/// Orders a record as JS iterates an object's keys: array indexes first, in numeric order, then the other keys in
/// insertion order. Id- and hash-keyed records need no reordering: no id or hash is an array index.
pub(crate) fn js_order<V>(map: &mut IndexMap<String, V>) {
    map.sort_by(|a, _, b, _| match (array_index(a), array_index(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

/// Reads a string-keyed record in JS iteration order ([`js_order`]), the order its diagnostics come in.
pub(crate) fn js_record<'de, D: Deserializer<'de>, V: Deserialize<'de>>(
    deserializer: D,
) -> Result<IndexMap<String, V>, D::Error> {
    let mut map = IndexMap::<String, V>::deserialize(deserializer)?;
    js_order(&mut map);
    Ok(map)
}

/// [`js_record`] for an optional record, with [`optional`]'s rules.
pub(crate) mod optional_js_record {
    use super::*;

    pub fn serialize<S: Serializer, V: Serialize>(
        value: &Option<IndexMap<String, V>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<IndexMap<String, V>>, D::Error> {
        js_record(deserializer).map(Some)
    }
}

/// A field that may be absent, `null`, or a value (zod's `T.nullable().optional()`).
pub type Nullable<T> = Option<Option<T>>;

/// Serde adapter for [`Nullable`] fields, used with `default` and `skip_serializing_if = "Option::is_none"`.
pub mod double_option {
    use super::*;

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(Some(inner)) => inner.serialize(serializer),
            _ => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    tagged_union! {
        enum Sample ("kind") {
            A = "a" { value: u32 },
            B = "b" {
                #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::optional")]
                inner: Option<Box<Sample>>,
                #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::de::double_option")]
                maybe: Nullable<u32>,
            },
            None = "none" {},
        }
    }

    fn parse(text: &str) -> Result<Sample, Decoded> {
        let value: Value = serde_json::from_str(text).unwrap();
        serde_path_to_error::deserialize(value).map_err(|e| decode(&e))
    }

    #[test]
    fn round_trips_tags_and_tri_states() {
        for text in [
            r#"{"kind":"a","value":3}"#,
            r#"{"kind":"none"}"#,
            r#"{"kind":"b","maybe":null}"#,
            r#"{"kind":"b","maybe":2}"#,
            r#"{"kind":"b"}"#,
        ] {
            let sample = parse(text).unwrap();
            assert_eq!(serde_json::to_string(&sample).unwrap(), text);
        }
        assert_eq!(
            parse(r#"{"kind":"b","maybe":null}"#).unwrap(),
            Sample::B {
                inner: None,
                maybe: Some(None)
            }
        );
    }

    #[test]
    fn keeps_the_path_through_nested_unions() {
        let error = parse(r#"{"kind":"b","inner":{"kind":"b","inner":{"kind":"a","value":-1}}}"#)
            .unwrap_err();
        let path: Vec<String> = error
            .path
            .iter()
            .map(|seg| match seg {
                PathSeg::Key(k) => k.to_string(),
                PathSeg::Index(i) => i.to_string(),
            })
            .collect();
        assert_eq!(path, ["inner", "inner", "value"]);
        assert_eq!(error.issues.len(), 1);
        assert_eq!(error.issues[0].code, codes::IR0001);
    }

    fn error_path(text: &str) -> String {
        parse(text)
            .unwrap_err()
            .path
            .into_iter()
            .collect::<crate::path::IrPath>()
            .to_string()
    }

    #[test]
    fn refuses_unknown_tags_and_fields() {
        assert!(parse(r#"{"kind":"c"}"#).is_err());
        assert!(parse(r#"{"kind":"none","extra":1}"#).is_err());
        assert!(parse(r#"{"value":1}"#).is_err());
    }

    #[test]
    fn paths_name_what_zod_names() {
        assert_eq!(
            error_path(r#"{"kind":"b","inner":{"kind":"a"}}"#),
            "inner.value"
        );
        assert_eq!(
            error_path(r#"{"kind":"b","inner":{"kind":"none","extra":1}}"#),
            "inner"
        );
        assert_eq!(
            error_path(r#"{"kind":"b","inner":{"kind":"c"}}"#),
            "inner.kind"
        );
        assert_eq!(
            error_path(r#"{"kind":"b","inner":{"value":1}}"#),
            "inner.kind"
        );
        assert_eq!(
            error_path(r#"{"kind":"b","inner":{"kind":7}}"#),
            "inner.kind"
        );
    }

    #[test]
    fn only_tri_states_take_null() {
        assert_eq!(error_path(r#"{"kind":"b","inner":null}"#), "inner");
        assert!(parse(r#"{"kind":"b","maybe":null}"#).is_ok());
    }

    #[test]
    fn carries_codes() {
        #[derive(Debug)]
        struct Off;
        impl<'de> Deserialize<'de> for Off {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(coded(
                    crate::diagnostics::codes::IR0007,
                    "Not a multiple of 0.001",
                ))
            }
        }
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Holder {
            x: Off,
        }
        let value = serde_json::json!({ "x": 1 });
        let error = serde_path_to_error::deserialize::<_, Holder>(value).unwrap_err();
        let decoded = decode(&error);
        assert_eq!(
            decoded.issues,
            [StructureIssue {
                code: codes::IR0007,
                detail: IssueDetail::Reason("Not a multiple of 0.001".into())
            }]
        );
        assert_eq!(decoded.path, vec![PathSeg::from("x")]);
    }
}
