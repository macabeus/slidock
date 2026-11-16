//! A canonical text of a decoded object, for comparing decoders and for debugging: a message is
//! `{<number>:<value>,…}` in field-number order, a repeated field `[<value>,…]`; numbers print as JavaScript's
//! `String(n)`, a `uint64` as `u<decimal>`, a string as `s<hex of its UTF-8>`, bytes as `b<hex>`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::iwa::registry::Archive;
use crate::iwa::schema::{
    FieldKind, MESSAGES, MessageSchema, WireError, WireReader, message_index,
};

/// The canonical text of an object, read back from the bytes `prost` writes for it.
pub fn dump(archive: &Archive) -> Result<String, WireError> {
    let bytes = archive.encode_to_vec();
    let index = message_index(archive.type_name()).unwrap_or(usize::MAX);
    let mut out = String::new();
    message(MESSAGES, index, &bytes, &mut out)?;
    Ok(out)
}

fn message(
    schema: &[MessageSchema],
    index: usize,
    bytes: &[u8],
    out: &mut String,
) -> Result<(), WireError> {
    let message = schema.get(index);
    let mut fields: BTreeMap<u32, (bool, Vec<String>)> = BTreeMap::new();
    let mut reader = WireReader::new(bytes);
    while !reader.done() {
        let tag = reader.uint32()?;
        let (number, wire_type) = (tag >> 3, tag & 7);
        let Some(field) = message.and_then(|m| m.field(number)) else {
            return Err(WireError::WireType(wire_type));
        };
        let values = &mut fields
            .entry(number)
            .or_insert_with(|| (field.repeated, Vec::new()))
            .1;
        let packed = wire_type == 2
            && !matches!(
                field.kind,
                FieldKind::String | FieldKind::Bytes | FieldKind::Message(_)
            );
        if packed {
            let length = reader.uint32()?;
            let mut run = WireReader::new(reader.take(length)?);
            while !run.done() {
                values.push(value(schema, field.kind, &mut run)?);
            }
        } else {
            values.push(value(schema, field.kind, &mut reader)?);
        }
    }
    out.push('{');
    for (i, (number, (repeated, values))) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{number}:");
        if *repeated {
            out.push('[');
            out.push_str(&values.join(","));
            out.push(']');
        } else if let Some(last) = values.last() {
            out.push_str(last);
        }
    }
    out.push('}');
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn fixed<const N: usize>(reader: &mut WireReader<'_>) -> Result<[u8; N], WireError> {
    reader
        .take(N as u32)?
        .try_into()
        .map_err(|_| WireError::FieldPastEnd)
}

fn value(
    schema: &[MessageSchema],
    kind: FieldKind,
    reader: &mut WireReader<'_>,
) -> Result<String, WireError> {
    Ok(match kind {
        FieldKind::Bool => (reader.varint64()? as u32 != 0).to_string(),
        FieldKind::Uint32 => (reader.varint64()? as u32).to_string(),
        FieldKind::Int32 | FieldKind::Enum => (reader.varint64()? as u32 as i32).to_string(),
        FieldKind::Uint64 => format!("u{}", reader.varint64()?),
        FieldKind::Fixed32 => u32::from_le_bytes(fixed(reader)?).to_string(),
        FieldKind::Float => {
            slidock_canon::to_js_string(f64::from(f32::from_le_bytes(fixed(reader)?)))
        }
        FieldKind::Double => slidock_canon::to_js_string(f64::from_le_bytes(fixed(reader)?)),
        FieldKind::String | FieldKind::Bytes => {
            let length = reader.uint32()?;
            let body = reader.take(length)?;
            let prefix = if kind == FieldKind::String { 's' } else { 'b' };
            format!("{prefix}{}", hex(body))
        }
        FieldKind::Message(child) => {
            let length = reader.uint32()?;
            let body = reader.take(length)?;
            let mut out = String::new();
            message(schema, child, body, &mut out)?;
            out
        }
    })
}
