//! Canonical JSON: object keys sorted by UTF-16 code unit, arrays in order, strings escaped as `JSON.stringify` escapes
//! them, and numbers in their canonical form ([`format_number`]). Equal values print equal bytes, so the output serves
//! as a cache key, a hash input and a golden file.

use std::fmt::Write as _;

use serde::Serialize;

use crate::error::JsonError;
use crate::number::format_number;
use crate::utf16;

/// A JSON value whose numbers are JavaScript numbers (`f64`).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Members in any order; printing sorts them, and a repeated key keeps its last value.
    Object(Vec<(String, Json)>),
}

/// Whitespace of the printed form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// No whitespace: cache keys and hashes.
    #[default]
    Compact,
    /// Two-space indent and a trailing newline: committed files, dumps and manifests.
    Indented,
}

impl From<&serde_json::Value> for Json {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Json::Null,
            serde_json::Value::Bool(b) => Json::Bool(*b),
            // Without `arbitrary_precision` every serde_json number has an f64 value.
            serde_json::Value::Number(n) => Json::Number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Json::String(s.clone()),
            serde_json::Value::Array(items) => Json::Array(items.iter().map(Json::from).collect()),
            serde_json::Value::Object(members) => Json::Object(
                members
                    .iter()
                    .map(|(k, v)| (k.clone(), Json::from(v)))
                    .collect(),
            ),
        }
    }
}

impl Json {
    /// Converts any `Serialize` value. Fails on a non-finite number, where `serde_json` would write `null`.
    pub fn from_serialize<T: Serialize + ?Sized>(value: &T) -> Result<Json, JsonError> {
        value.serialize(JsonSerializer)
    }

    /// The canonical text.
    pub fn to_canonical(&self, layout: Layout) -> Result<String, JsonError> {
        let mut out = String::new();
        self.write(layout, 0, &mut out)?;
        if layout == Layout::Indented {
            out.push('\n');
        }
        Ok(out)
    }

    fn write(&self, layout: Layout, depth: usize, out: &mut String) -> Result<(), JsonError> {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(n) => out.push_str(&format_number(*n)?),
            Json::String(s) => write_json_string(s, out),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return Ok(());
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    separate(layout, depth + 1, i == 0, out);
                    item.write(layout, depth + 1, out)?;
                }
                close(layout, depth, out);
                out.push(']');
            }
            Json::Object(members) => {
                let members = sorted_members(members);
                if members.is_empty() {
                    out.push_str("{}");
                    return Ok(());
                }
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    separate(layout, depth + 1, i == 0, out);
                    write_json_string(key, out);
                    out.push_str(if layout == Layout::Indented {
                        ": "
                    } else {
                        ":"
                    });
                    value.write(layout, depth + 1, out)?;
                }
                close(layout, depth, out);
                out.push('}');
            }
        }
        Ok(())
    }
}

/// Members sorted by key in UTF-16 order, each key once with its last value.
fn sorted_members(members: &[(String, Json)]) -> Vec<&(String, Json)> {
    let mut sorted: Vec<&(String, Json)> = members.iter().collect();
    sorted.sort_by(|a, b| utf16::cmp(&a.0, &b.0));
    let mut unique: Vec<&(String, Json)> = Vec::with_capacity(sorted.len());
    for member in sorted {
        match unique.last_mut() {
            Some(last) if last.0 == member.0 => *last = member,
            _ => unique.push(member),
        }
    }
    unique
}

fn separate(layout: Layout, depth: usize, first: bool, out: &mut String) {
    if !first {
        out.push(',');
    }
    if layout == Layout::Indented {
        out.push('\n');
        indent(depth, out);
    }
}

fn close(layout: Layout, depth: usize, out: &mut String) {
    if layout == Layout::Indented {
        out.push('\n');
        indent(depth, out);
    }
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// `JSON.stringify(s)`: quotes, `\"`, `\\`, `\b \f \n \r \t`, other C0 controls as lowercase `\u00xx`; every
/// other character (DEL, U+2028, U+2029, non-ASCII) as is.
pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\u{20}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Canonical JSON of any `Serialize` value.
pub fn to_canonical_json<T: Serialize + ?Sized>(
    value: &T,
    layout: Layout,
) -> Result<String, JsonError> {
    Json::from_serialize(value)?.to_canonical(layout)
}

/// Canonical JSON of a parsed `serde_json` value.
pub fn value_to_canonical_json(
    value: &serde_json::Value,
    layout: Layout,
) -> Result<String, JsonError> {
    Json::from(value).to_canonical(layout)
}

/// Builds a [`Json`] tree with serde's data model as `serde_json` maps it (externally tagged enums, `None` and
/// unit as `null`), keeping every number as an `f64`.
struct JsonSerializer;

fn number<T: Into<f64>>(n: T) -> Result<Json, JsonError> {
    Ok(Json::Number(n.into()))
}

fn tagged(variant: &'static str, value: Json) -> Json {
    Json::Object(vec![(variant.to_owned(), value)])
}

// Integers become JavaScript numbers, so values beyond 2^53 round as `JSON.parse` would round them.
impl serde::Serializer for JsonSerializer {
    type Ok = Json;
    type Error = JsonError;
    type SerializeSeq = SeqBuilder;
    type SerializeTuple = SeqBuilder;
    type SerializeTupleStruct = SeqBuilder;
    type SerializeTupleVariant = SeqBuilder;
    type SerializeMap = MapBuilder;
    type SerializeStruct = MapBuilder;
    type SerializeStructVariant = MapBuilder;

    fn serialize_bool(self, v: bool) -> Result<Json, JsonError> {
        Ok(Json::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_i16(self, v: i16) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_i32(self, v: i32) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_i64(self, v: i64) -> Result<Json, JsonError> {
        Ok(Json::Number(v as f64))
    }
    fn serialize_i128(self, v: i128) -> Result<Json, JsonError> {
        Ok(Json::Number(v as f64))
    }
    fn serialize_u8(self, v: u8) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_u16(self, v: u16) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_u32(self, v: u32) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_u64(self, v: u64) -> Result<Json, JsonError> {
        Ok(Json::Number(v as f64))
    }
    fn serialize_u128(self, v: u128) -> Result<Json, JsonError> {
        Ok(Json::Number(v as f64))
    }
    fn serialize_f32(self, v: f32) -> Result<Json, JsonError> {
        number(v)
    }
    fn serialize_f64(self, v: f64) -> Result<Json, JsonError> {
        if v.is_finite() {
            Ok(Json::Number(v))
        } else {
            Err(crate::NumberError::NonFinite(v).into())
        }
    }
    fn serialize_char(self, v: char) -> Result<Json, JsonError> {
        Ok(Json::String(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Json, JsonError> {
        Ok(Json::String(v.to_owned()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Json, JsonError> {
        Ok(Json::Array(
            v.iter().map(|b| Json::Number(f64::from(*b))).collect(),
        ))
    }
    fn serialize_none(self) -> Result<Json, JsonError> {
        Ok(Json::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Json, JsonError> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Json, JsonError> {
        Ok(Json::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Json, JsonError> {
        Ok(Json::Null)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Json, JsonError> {
        Ok(Json::String(variant.to_owned()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Json, JsonError> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Json, JsonError> {
        Ok(tagged(variant, value.serialize(self)?))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqBuilder, JsonError> {
        Ok(SeqBuilder {
            variant: None,
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqBuilder, JsonError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<SeqBuilder, JsonError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SeqBuilder, JsonError> {
        Ok(SeqBuilder {
            variant: Some(variant),
            items: Vec::with_capacity(len),
        })
    }
    fn serialize_map(self, len: Option<usize>) -> Result<MapBuilder, JsonError> {
        Ok(MapBuilder {
            variant: None,
            members: Vec::with_capacity(len.unwrap_or(0)),
            key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<MapBuilder, JsonError> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<MapBuilder, JsonError> {
        Ok(MapBuilder {
            variant: Some(variant),
            members: Vec::with_capacity(len),
            key: None,
        })
    }
}

struct SeqBuilder {
    variant: Option<&'static str>,
    items: Vec<Json>,
}

impl SeqBuilder {
    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        self.items.push(value.serialize(JsonSerializer)?);
        Ok(())
    }

    fn finish(self) -> Json {
        let array = Json::Array(self.items);
        match self.variant {
            Some(variant) => tagged(variant, array),
            None => array,
        }
    }
}

impl serde::ser::SerializeSeq for SeqBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        self.push(value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTuple for SeqBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        self.push(value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTupleStruct for SeqBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        self.push(value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTupleVariant for SeqBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        self.push(value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

struct MapBuilder {
    variant: Option<&'static str>,
    members: Vec<(String, Json)>,
    key: Option<String>,
}

impl MapBuilder {
    fn finish(self) -> Json {
        let object = Json::Object(self.members);
        match self.variant {
            Some(variant) => tagged(variant, object),
            None => object,
        }
    }

    fn field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), JsonError> {
        self.members
            .push((key.to_owned(), value.serialize(JsonSerializer)?));
        Ok(())
    }
}

impl serde::ser::SerializeMap for MapBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), JsonError> {
        self.key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), JsonError> {
        let key = self
            .key
            .take()
            .ok_or(JsonError::Unsupported("map value without a key"))?;
        self.members.push((key, value.serialize(JsonSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeStruct for MapBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), JsonError> {
        self.field(key, value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeStructVariant for MapBuilder {
    type Ok = Json;
    type Error = JsonError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), JsonError> {
        self.field(key, value)
    }
    fn end(self) -> Result<Json, JsonError> {
        Ok(self.finish())
    }
}

/// Object keys: strings, characters, unit variants and integers (printed in decimal), as `serde_json` accepts.
struct KeySerializer;

macro_rules! integer_keys {
    ($($method:ident: $ty:ty),*) => {
        $(fn $method(self, v: $ty) -> Result<String, JsonError> {
            Ok(v.to_string())
        })*
    };
}

impl serde::Serializer for KeySerializer {
    type Ok = String;
    type Error = JsonError;
    type SerializeSeq = serde::ser::Impossible<String, JsonError>;
    type SerializeTuple = serde::ser::Impossible<String, JsonError>;
    type SerializeTupleStruct = serde::ser::Impossible<String, JsonError>;
    type SerializeTupleVariant = serde::ser::Impossible<String, JsonError>;
    type SerializeMap = serde::ser::Impossible<String, JsonError>;
    type SerializeStruct = serde::ser::Impossible<String, JsonError>;
    type SerializeStructVariant = serde::ser::Impossible<String, JsonError>;

    integer_keys!(
        serialize_i8: i8, serialize_i16: i16, serialize_i32: i32, serialize_i64: i64, serialize_i128: i128,
        serialize_u8: u8, serialize_u16: u16, serialize_u32: u32, serialize_u64: u64, serialize_u128: u128
    );

    fn serialize_str(self, v: &str) -> Result<String, JsonError> {
        Ok(v.to_owned())
    }
    fn serialize_char(self, v: char) -> Result<String, JsonError> {
        Ok(v.to_string())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<String, JsonError> {
        Ok(variant.to_owned())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, JsonError> {
        value.serialize(self)
    }
    fn serialize_bool(self, _v: bool) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_f32(self, _v: f32) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_f64(self, _v: f64) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_none(self) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_unit(self) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, JsonError> {
        Err(JsonError::KeyNotString)
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, JsonError> {
        Err(JsonError::KeyNotString)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Sample {
        zeta: f64,
        alpha: Vec<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        absent: Option<u8>,
        present: Option<u8>,
        nested: BTreeMap<String, bool>,
        empty_list: Vec<u8>,
        empty_map: BTreeMap<String, u8>,
        kind: Kind,
    }

    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    enum Kind {
        Pt { value: f64 },
    }

    fn sample() -> Sample {
        Sample {
            zeta: 7.2,
            alpha: vec![1, 2],
            absent: None,
            present: None,
            nested: BTreeMap::from([("b".into(), true), ("a".into(), false)]),
            empty_list: vec![],
            empty_map: BTreeMap::new(),
            kind: Kind::Pt { value: -0.0 },
        }
    }

    #[test]
    fn compact_sorts_keys_and_prints_js_numbers() {
        assert_eq!(
            to_canonical_json(&sample(), Layout::Compact).unwrap(),
            r#"{"alpha":[1,2],"emptyList":[],"emptyMap":{},"kind":{"kind":"pt","value":0},"nested":{"a":false,"b":true},"present":null,"zeta":7.2}"#
        );
    }

    #[test]
    fn indented_uses_two_spaces_and_a_trailing_newline() {
        let expected = "{\n  \"alpha\": [\n    1,\n    2\n  ],\n  \"emptyList\": [],\n  \"emptyMap\": {},\n  \
                        \"kind\": {\n    \"kind\": \"pt\",\n    \"value\": 0\n  },\n  \"nested\": {\n    \
                        \"a\": false,\n    \"b\": true\n  },\n  \"present\": null,\n  \"zeta\": 7.2\n}\n";
        assert_eq!(
            to_canonical_json(&sample(), Layout::Indented).unwrap(),
            expected
        );
        assert_eq!(to_canonical_json(&1.5, Layout::Indented).unwrap(), "1.5\n");
    }

    #[test]
    fn keys_sort_by_utf16_code_units() {
        let value = serde_json::json!({ "😀": 1, "\u{FF5E}": 2, "a": 3, "B": 4 });
        assert_eq!(
            value_to_canonical_json(&value, Layout::Compact).unwrap(),
            "{\"B\":4,\"a\":3,\"😀\":1,\"\u{FF5E}\":2}"
        );
    }

    #[test]
    fn a_repeated_key_keeps_its_last_value() {
        let json = Json::Object(vec![
            ("b".into(), Json::Number(1.0)),
            ("a".into(), Json::Null),
            ("b".into(), Json::Number(2.0)),
        ]);
        assert_eq!(
            json.to_canonical(Layout::Compact).unwrap(),
            r#"{"a":null,"b":2}"#
        );
    }

    #[test]
    fn refuses_numbers_without_a_canonical_form() {
        assert!(matches!(
            to_canonical_json(&f64::NAN, Layout::Compact),
            Err(JsonError::Number(_))
        ));
        assert!(matches!(
            to_canonical_json(&[1e21], Layout::Compact),
            Err(JsonError::Number(_))
        ));
        assert!(matches!(
            to_canonical_json(&Some(1e-7), Layout::Compact),
            Err(JsonError::Number(_))
        ));
    }

    #[test]
    fn refuses_non_string_keys() {
        let map = BTreeMap::from([(vec![1u8], 1u8)]);
        assert_eq!(
            to_canonical_json(&map, Layout::Compact),
            Err(JsonError::KeyNotString)
        );
        let ints = BTreeMap::from([(10u32, true), (2u32, false)]);
        assert_eq!(
            to_canonical_json(&ints, Layout::Compact).unwrap(),
            r#"{"10":true,"2":false}"#
        );
    }

    #[test]
    fn serde_data_model_maps_like_serde_json() {
        #[derive(Serialize)]
        enum E {
            Unit,
            New(u8),
            Tuple(u8, u8),
            Struct { x: u8 },
        }
        #[derive(Serialize)]
        struct Newtype(&'static str);
        let value = (
            E::Unit,
            E::New(1),
            E::Tuple(1, 2),
            E::Struct { x: 3 },
            Newtype("n"),
            (),
            'c',
        );
        let ours = to_canonical_json(&value, Layout::Compact).unwrap();
        let theirs =
            value_to_canonical_json(&serde_json::to_value(&value).unwrap(), Layout::Compact)
                .unwrap();
        assert_eq!(ours, theirs);
        assert_eq!(
            ours,
            r#"["Unit",{"New":1},{"Tuple":[1,2]},{"Struct":{"x":3}},"n",null,"c"]"#
        );
    }

    #[test]
    fn escapes_strings_as_json_stringify() {
        let mut out = String::new();
        write_json_string("\"\\\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}\u{2028}/é", &mut out);
        assert_eq!(
            out,
            "\"\\\"\\\\\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}\u{2028}/é\""
        );
    }
}
