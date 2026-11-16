//! Messages written from JSON by the crate's schema tables, as protobufjs's `encode(fromObject(…))` writes them, so a
//! fixture reads as the object it describes: a key is a field name, a nested object a message, an array a repeated
//! field. Keys the schema does not hold are left out. Float fields also take `"Infinity"`, `"-Infinity"` and `"NaN"`,
//! which JSON cannot write; `uint64` fields take numbers or decimal strings; `bytes` fields take arrays of bytes.

use serde_json::Value;
use slidock_keynote::iwa::schema::{FieldKind, FieldSchema, message_index, messages};

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap(),
        Value::String(s) => match s.as_str() {
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            "NaN" => f64::NAN,
            other => other
                .parse()
                .unwrap_or_else(|_| panic!("not a number: {other}")),
        },
        Value::Bool(b) => f64::from(u8::from(*b)),
        other => panic!("not a number: {other}"),
    }
}

fn integer(value: &Value) -> u64 {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().map(|i| i as u64))
            .unwrap_or_else(|| n.as_f64().unwrap() as i64 as u64),
        Value::String(s) => s.parse().unwrap_or_else(|_| panic!("not an integer: {s}")),
        Value::Bool(b) => u64::from(*b),
        other => panic!("not an integer: {other}"),
    }
}

fn value(out: &mut Vec<u8>, field: &FieldSchema, value: &Value) {
    let tag = |out: &mut Vec<u8>, wire: u64| varint(out, (u64::from(field.number) << 3) | wire);
    match field.kind {
        FieldKind::Bool => {
            tag(out, 0);
            let truthy = match value {
                Value::Bool(b) => *b,
                other => number(other) != 0.0,
            };
            varint(out, u64::from(truthy));
        }
        FieldKind::Uint32 => {
            tag(out, 0);
            varint(out, integer(value) & 0xffff_ffff);
        }
        FieldKind::Int32 | FieldKind::Enum => {
            tag(out, 0);
            // Sign-extended to 64 bits, as protobuf writes a negative int32.
            varint(out, i64::from(number(value) as i32) as u64);
        }
        FieldKind::Uint64 => {
            tag(out, 0);
            varint(out, integer(value));
        }
        FieldKind::Fixed32 => {
            tag(out, 5);
            out.extend((integer(value) as u32).to_le_bytes());
        }
        FieldKind::Float => {
            tag(out, 5);
            out.extend((number(value) as f32).to_le_bytes());
        }
        FieldKind::Double => {
            tag(out, 1);
            out.extend(number(value).to_le_bytes());
        }
        FieldKind::String => {
            let text = value
                .as_str()
                .unwrap_or_else(|| panic!("not a string: {value}"));
            tag(out, 2);
            varint(out, text.len() as u64);
            out.extend(text.as_bytes());
        }
        FieldKind::Bytes => {
            let bytes: Vec<u8> = value
                .as_array()
                .unwrap_or_else(|| panic!("not bytes: {value}"))
                .iter()
                .map(|b| integer(b) as u8)
                .collect();
            tag(out, 2);
            varint(out, bytes.len() as u64);
            out.extend(bytes);
        }
        FieldKind::Message(index) => {
            let body = encode_at(index, value);
            tag(out, 2);
            varint(out, body.len() as u64);
            out.extend(body);
        }
    }
}

fn encode_at(index: usize, message: &Value) -> Vec<u8> {
    let schema = &messages()[index];
    let object = message
        .as_object()
        .unwrap_or_else(|| panic!("{}: not an object: {message}", schema.name));
    let mut out = Vec::new();
    for field in schema.fields {
        let Some(present) = object.get(field.name).filter(|v| !v.is_null()) else {
            continue;
        };
        if field.repeated {
            for item in present
                .as_array()
                .unwrap_or_else(|| panic!("{}.{}: not an array", schema.name, field.name))
            {
                value(&mut out, field, item);
            }
        } else {
            value(&mut out, field, present);
        }
    }
    out
}

/// A message of `type_name` written from JSON.
pub fn encode(type_name: &str, message: &Value) -> Vec<u8> {
    let index = message_index(type_name).unwrap_or_else(|| panic!("no message {type_name}"));
    encode_at(index, message)
}
