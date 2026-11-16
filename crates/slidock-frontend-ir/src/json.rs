//! JSON read as `JSON.parse` reads it, within what a Rust string holds. A lone surrogate escape (`\ud800`), which no
//! Rust string can hold, reads as U+FFFD, the character a UTF-8 writer puts in its place; it stays one UTF-16 unit, so
//! lengths and cuts count the same. HTML outputs hold the character's UTF-8 bytes, and JSON outputs (the IR dump,
//! `presentation.json`) hold U+FFFD. An object lists its array-index keys (`9`, `10`) first, in numeric order, then the
//! rest as written, the order `Object.entries` visits. An object that repeats a member is refused (a data error,
//! [`serde_json::Error::is_data`]), where `JSON.parse` keeps the last value: no repeat can hide a value the reader
//! checks.

use std::borrow::Cow;
use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// The JSON `text` holds, as `JSON.parse` reads it.
pub(crate) fn parse(text: &str) -> Result<Value, serde_json::Error> {
    let text = without_lone_surrogates(text);
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    let mut value = UniqueMembers.deserialize(&mut deserializer)?;
    deserializer.end()?;
    js_order(&mut value);
    Ok(value)
}

/// A JSON value whose objects name each member once; otherwise as `Value` reads it.
struct UniqueMembers;

impl<'de> DeserializeSeed<'de> for UniqueMembers {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueMembers {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(UniqueMembers)? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(A::Error::custom(format_args!(
                    "A JSON object repeats the member {key:?}"
                )));
            }
            let value = map.next_value_seed(UniqueMembers)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

/// The code unit of the `\uXXXX` escape at `at`, if one starts there.
fn escaped_unit(bytes: &[u8], at: usize) -> Option<u16> {
    let hex = bytes.get(at..at + 6)?.strip_prefix(b"\\u")?;
    if !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()
}

/// `text` with each escape of a surrogate outside a pair written `�`, which keeps every offset.
fn without_lone_surrogates(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut in_string = false;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'"' => {
                in_string = !in_string;
                at += 1;
            }
            b'\\' if in_string => match escaped_unit(bytes, at) {
                Some(0xD800..=0xDBFF)
                    if matches!(escaped_unit(bytes, at + 6), Some(0xDC00..=0xDFFF)) =>
                {
                    at += 12;
                }
                Some(0xD800..=0xDFFF) => {
                    out.push_str(text.get(copied..at).unwrap_or_default());
                    out.push_str("\\ufffd");
                    at += 6;
                    copied = at;
                }
                Some(_) => at += 6,
                // Any other escape is two characters; the parser reports an invalid one.
                None => at += 2,
            },
            _ => at += 1,
        }
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    Cow::Owned(out)
}

/// The array index a key stands for: a canonical decimal (`0`, or no leading zero) below 2^32 - 1.
fn array_index(key: &str) -> Option<u32> {
    let canonical =
        key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
    key.parse::<u32>()
        .ok()
        .filter(|&index| canonical && index != u32::MAX)
}

/// Puts every object of `value` in `Object.entries` order.
fn js_order(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(js_order),
        Value::Object(object) => {
            object.values_mut().for_each(js_order);
            if object.keys().any(|key| array_index(key).is_some()) {
                let mut entries: Vec<(String, Value)> =
                    std::mem::take(object).into_iter().collect();
                // Stable: keys that are not indexes keep the order they were written in.
                entries.sort_by_key(|(key, _)| array_index(key).map_or((1, 0), |index| (0, index)));
                *object = entries.into_iter().collect();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::*;

    fn keys(value: &Value) -> Vec<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn reads_a_lone_surrogate_escape_as_the_replacement_character() {
        for (text, expected) in [
            (r#""Hi \ud800 x""#, "Hi \u{FFFD} x"),
            (r#""\uDC00\uD800""#, "\u{FFFD}\u{FFFD}"),
            (r#""\ud83d""#, "\u{FFFD}"),
            (r#""\ud83d😀""#, "\u{FFFD}\u{1F600}"),
            (r#""\\ud800""#, "\\ud800"),
            (r#""\"\ud800""#, "\"\u{FFFD}"),
            (r#""😀 é \u0000""#, "\u{1F600} é \u{0}"),
        ] {
            assert_eq!(parse(text).unwrap(), Value::from(expected), "{text}");
        }
        assert!(matches!(
            without_lone_surrogates(r#"{"a":"😀"}"#),
            Cow::Borrowed(_)
        ));
        // Offsets stay, so the parser's error positions match the text.
        let text = r#"["\ud800", nope]"#;
        assert_eq!(without_lone_surrogates(text).len(), text.len());
        let paired = r#"["\u0041", nope]"#;
        assert_eq!(
            parse(text).unwrap_err().column(),
            parse(paired).unwrap_err().column()
        );
        // Outside a string, a backslash is the parser's to refuse.
        assert!(parse(r#"[\ud800]"#).is_err());
    }

    #[test]
    fn refuses_an_object_that_repeats_a_member() {
        for text in [
            r#"{"a":1,"a":1}"#,
            r#"{"a":1,"b":{"c":[{"d":0,"d":0}]}}"#,
            r#"[{"\u0061":1,"a":2}]"#,
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.is_data(), "{text}: {error}");
            assert!(
                error
                    .to_string()
                    .starts_with("A JSON object repeats the member"),
                "{error}"
            );
        }
        // Each object has its own members; the same name in two objects, or as a value, is no repeat.
        let value = parse(r#"{"a":{"a":"a"},"b":[{"a":1},{"a":2}],"A":0}"#).unwrap();
        assert_eq!(keys(&value), ["a", "b", "A"]);
        assert_eq!(value["b"][1]["a"], 2);
        // Values read as serde_json reads them, and trailing text is still refused.
        let text =
            r#"[1,-1,18446744073709551615,-9223372036854775808,1e300,0.1,1.5e-7,true,null,"x"]"#;
        assert_eq!(
            parse(text).unwrap(),
            serde_json::from_str::<Value>(text).unwrap()
        );
        assert!(parse(r#"{"a":1} x"#).unwrap_err().is_syntax());
    }

    #[test]
    fn lists_array_index_keys_first_in_numeric_order() {
        let value = parse(
            r#"{"b":1,"10":{"z":0,"2":0,"a":0},"9":2,"01":3,"4294967295":4,"4294967294":5,"a":6,"0":7}"#,
        )
        .unwrap();
        assert_eq!(
            keys(&value),
            ["0", "9", "10", "4294967294", "b", "01", "4294967295", "a"]
        );
        assert_eq!(keys(&value["10"]), ["2", "z", "a"]);
        let nested = parse(r#"[{"x":{"3":0,"1":0}}]"#).unwrap();
        assert_eq!(keys(&nested[0]["x"]), ["1", "3"]);
    }
}
