//! The wire decoder in front of `prost`. It reads a payload by the trimmed schema's field tables, as a schema-driven
//! decoder does, and writes it back as canonical protobuf that `prost` decodes to the same values:
//! - fields outside the schema, groups, and fields written with another wire type than the schema's are dropped;
//! - packed and unpacked runs of repeated numbers are both read;
//! - a singular field set twice keeps its last value, a message too (replaced, not merged);
//! - `bool`, `uint32` and `int32` read the varint's low 32 bits, strings decode as `TextDecoder` does (lossy, a
//!   leading byte order mark dropped);
//! - nesting is capped at [`MAX_DEPTH`], and every message and value counts against a limit as it is built.

use std::borrow::Cow;

use thiserror::Error;

/// How deep messages, and groups being skipped, nest in a payload this decoder reads.
pub const MAX_DEPTH: usize = 64;

const VARINT: u32 = 0;
const FIXED64: u32 = 1;
const DELIMITED: u32 = 2;
const START_GROUP: u32 = 3;
const END_GROUP: u32 = 4;
const FIXED32: u32 = 5;

/// What a field holds, as the schema declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Bool,
    Uint32,
    Int32,
    Enum,
    Uint64,
    Fixed32,
    Float,
    Double,
    String,
    Bytes,
    /// A message, by its index in the schema's message table.
    Message(usize),
}

impl FieldKind {
    /// What one element of a repeated field of the kind holds in memory, the same on every target: a number its
    /// width, a string or bytes its own 24 (its contents are counted apart). A message element costs its
    /// [`MessageSchema::cost`].
    const fn element_cost(self) -> u64 {
        match self {
            FieldKind::Bool => 1,
            FieldKind::Uint32
            | FieldKind::Int32
            | FieldKind::Enum
            | FieldKind::Fixed32
            | FieldKind::Float => 4,
            FieldKind::Uint64 | FieldKind::Double => 8,
            FieldKind::String | FieldKind::Bytes => 24,
            FieldKind::Message(_) => 0,
        }
    }

    /// The wire type the kind is written with, unpacked.
    const fn wire_type(self) -> u32 {
        match self {
            FieldKind::Bool
            | FieldKind::Uint32
            | FieldKind::Int32
            | FieldKind::Enum
            | FieldKind::Uint64 => VARINT,
            FieldKind::Fixed32 | FieldKind::Float => FIXED32,
            FieldKind::Double => FIXED64,
            FieldKind::String | FieldKind::Bytes | FieldKind::Message(_) => DELIMITED,
        }
    }
}

/// A field of a schema message.
#[derive(Clone, Copy, Debug)]
pub struct FieldSchema {
    pub number: u32,
    pub name: &'static str,
    pub kind: FieldKind,
    pub repeated: bool,
}

/// A message of the schema.
#[derive(Clone, Copy, Debug)]
pub struct MessageSchema {
    /// The full name, `KN.SlideArchive`.
    pub name: &'static str,
    /// What a decoded message costs in memory, the same on every target: 24 bytes, plus 16 per scalar field, 24 per
    /// string, bytes or repeated field, and an inline (singular) message field's own cost.
    pub cost: u64,
    /// Sorted by number.
    pub fields: &'static [FieldSchema],
}

impl MessageSchema {
    /// The field of a number.
    pub fn field(&self, number: u32) -> Option<&FieldSchema> {
        self.fields
            .binary_search_by_key(&number, |field| field.number)
            .ok()
            .and_then(|at| self.fields.get(at))
    }
}

include!(concat!(env!("OUT_DIR"), "/schema_tables.rs"));

/// Every message of the trimmed schema, sorted by full name; [`FieldKind::Message`] indexes it.
pub fn messages() -> &'static [MessageSchema] {
    MESSAGES
}

/// The index of a full name in [`messages`].
pub fn message_index(name: &str) -> Option<usize> {
    MESSAGES.binary_search_by(|m| m.name.cmp(name)).ok()
}

/// Why a payload does not decode.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WireError {
    #[error("a varint runs past the end")]
    VarintPastEnd,
    #[error("a varint is longer than 10 bytes")]
    VarintTooLong,
    #[error("a field runs past the end")]
    FieldPastEnd,
    #[error("wire type {0}")]
    WireType(u32),
    #[error("groups nest deeper than {MAX_DEPTH}")]
    GroupsTooDeep,
    #[error("messages nest deeper than {MAX_DEPTH}")]
    MessagesTooDeep,
    /// The payload holds more messages than the decode was allowed.
    #[error("more than {0} messages")]
    TooManyMessages(u64),
    /// The payload decodes past the memory the decode was allowed.
    #[error("more than {0} bytes decoded")]
    TooManyBytes(u64),
}

/// What a decode may build: messages (nested ones included), and their cost in memory
/// ([`MessageSchema::cost`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeLimit {
    pub messages: u64,
    pub bytes: u64,
}

impl DecodeLimit {
    pub const NONE: DecodeLimit = DecodeLimit {
        messages: u64::MAX,
        bytes: u64::MAX,
    };
}

/// A cursor over protobuf wire data that fails on any read past its end.
#[derive(Clone, Debug)]
pub(crate) struct WireReader<'a> {
    bytes: &'a [u8],
    pub pos: usize,
}

impl<'a> WireReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        WireReader { bytes, pos: 0 }
    }

    pub(crate) fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// A varint of at most 10 bytes, its bits past 64 dropped.
    pub(crate) fn varint64(&mut self) -> Result<u64, WireError> {
        let mut value: u64 = 0;
        for shift in (0..70).step_by(7) {
            let byte = *self.bytes.get(self.pos).ok_or(WireError::VarintPastEnd)?;
            self.pos += 1;
            if let Some(bits) = u64::from(byte & 0x7f).checked_shl(shift) {
                value |= bits;
            }
            if byte < 0x80 {
                return Ok(value);
            }
        }
        Err(WireError::VarintTooLong)
    }

    /// A varint's low 32 bits.
    pub(crate) fn uint32(&mut self) -> Result<u32, WireError> {
        Ok(self.varint64()? as u32)
    }

    /// The next `length` bytes.
    pub(crate) fn take(&mut self, length: u32) -> Result<&'a [u8], WireError> {
        let end = self
            .pos
            .checked_add(length as usize)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(WireError::FieldPastEnd)?;
        let out = self
            .bytes
            .get(self.pos..end)
            .ok_or(WireError::FieldPastEnd)?;
        self.pos = end;
        Ok(out)
    }

    fn skip(&mut self, wire_type: u32, depth: usize) -> Result<(), WireError> {
        match wire_type {
            VARINT => self.varint64().map(|_| ()),
            FIXED64 => self.take(8).map(|_| ()),
            DELIMITED => {
                let length = self.uint32()?;
                self.take(length).map(|_| ())
            }
            FIXED32 => self.take(4).map(|_| ()),
            START_GROUP => {
                if depth > MAX_DEPTH {
                    return Err(WireError::GroupsTooDeep);
                }
                loop {
                    let tag = self.uint32()?;
                    if tag & 7 == END_GROUP {
                        return Ok(());
                    }
                    self.skip(tag & 7, depth + 1)?;
                }
            }
            other => Err(WireError::WireType(other)),
        }
    }
}

/// A payload as canonical protobuf, how many messages it holds (itself included) and what they cost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sanitized {
    pub bytes: Vec<u8>,
    pub messages: u64,
    pub cost: u64,
}

/// `bytes` read as the message `index` of `schema`, written back canonically. Each message and value counts against
/// `limit` as it is built, so a payload that names one object thousands of times stops at the limit. A message
/// costs its [`MessageSchema::cost`] where it stands alone (the payload's, a repeated field's element); a singular
/// message field is inline in its parent's cost. Each element of a repeated number, string or bytes field adds its
/// [`FieldKind::element_cost`], and each string or bytes value its length.
pub(crate) fn sanitize(
    schema: &[MessageSchema],
    index: usize,
    bytes: &[u8],
    limit: DecodeLimit,
) -> Result<Sanitized, WireError> {
    let mut decoder = Decoder {
        schema,
        messages: 0,
        cost: 0,
        limit,
    };
    let bytes = decoder.message(index, bytes, 0, false)?;
    Ok(Sanitized {
        bytes,
        messages: decoder.messages,
        cost: decoder.cost,
    })
}

struct Decoder<'s> {
    schema: &'s [MessageSchema],
    messages: u64,
    cost: u64,
    limit: DecodeLimit,
}

/// A scalar as it is written back.
enum Value<'a> {
    Varint(u64),
    Fixed32(&'a [u8]),
    Fixed64(&'a [u8]),
    Delimited(Cow<'a, [u8]>),
}

impl Decoder<'_> {
    fn message(
        &mut self,
        index: usize,
        bytes: &[u8],
        depth: usize,
        inline: bool,
    ) -> Result<Vec<u8>, WireError> {
        if depth > MAX_DEPTH {
            return Err(WireError::MessagesTooDeep);
        }
        self.messages += 1;
        if self.messages > self.limit.messages {
            return Err(WireError::TooManyMessages(self.limit.messages));
        }
        let message = self.schema.get(index);
        if !inline {
            self.charge(message.map_or(0, |m| m.cost))?;
        }
        let mut out = Writer::default();
        let mut reader = WireReader::new(bytes);
        while !reader.done() {
            let tag = reader.uint32()?;
            let wire_type = tag & 7;
            let number = tag >> 3;
            let Some(field) = message.and_then(|m| m.field(number)) else {
                reader.skip(wire_type, 0)?;
                continue;
            };
            match field.kind {
                FieldKind::Message(child) if wire_type == DELIMITED => {
                    let length = reader.uint32()?;
                    let body = reader.take(length)?;
                    let child = self.message(child, body, depth + 1, !field.repeated)?;
                    out.field(field, |buf| {
                        put_varint(buf, u64::from(number << 3 | DELIMITED));
                        put_varint(buf, child.len() as u64);
                        buf.extend_from_slice(&child);
                    });
                }
                FieldKind::Message(_) => reader.skip(wire_type, 0)?,
                kind if wire_type == kind.wire_type() => {
                    let value = scalar(&mut reader, kind)?;
                    self.charge(held(field, &value))?;
                    out.field(field, |buf| put_value(buf, number, &value));
                }
                kind if field.repeated && wire_type == DELIMITED => {
                    // A packed run of numbers.
                    let length = reader.uint32()?;
                    let mut packed = WireReader::new(reader.take(length)?);
                    while !packed.done() {
                        let value = scalar(&mut packed, kind)?;
                        self.charge(held(field, &value))?;
                        out.field(field, |buf| put_value(buf, number, &value));
                    }
                }
                _ => reader.skip(wire_type, 0)?,
            }
        }
        Ok(out.finish())
    }

    fn charge(&mut self, cost: u64) -> Result<(), WireError> {
        self.cost = self.cost.saturating_add(cost);
        if self.cost > self.limit.bytes {
            return Err(WireError::TooManyBytes(self.limit.bytes));
        }
        Ok(())
    }
}

/// What a value of `field` adds to its message's cost: a repeated field's element, and a string's or bytes' length.
fn held(field: &FieldSchema, value: &Value<'_>) -> u64 {
    let element = if field.repeated {
        field.kind.element_cost()
    } else {
        0
    };
    let contents = match value {
        Value::Delimited(bytes) => bytes.len() as u64,
        Value::Varint(_) | Value::Fixed32(_) | Value::Fixed64(_) => 0,
    };
    element.saturating_add(contents)
}

fn scalar<'a>(reader: &mut WireReader<'a>, kind: FieldKind) -> Result<Value<'a>, WireError> {
    Ok(match kind {
        FieldKind::Bool => Value::Varint(u64::from(reader.varint64()? as u32 != 0)),
        FieldKind::Uint32 => Value::Varint(u64::from(reader.varint64()? as u32)),
        FieldKind::Int32 | FieldKind::Enum => {
            Value::Varint(i64::from(reader.varint64()? as u32 as i32) as u64)
        }
        FieldKind::Uint64 => Value::Varint(reader.varint64()?),
        FieldKind::Fixed32 | FieldKind::Float => Value::Fixed32(reader.take(4)?),
        FieldKind::Double => Value::Fixed64(reader.take(8)?),
        FieldKind::String => {
            let length = reader.uint32()?;
            Value::Delimited(text(reader.take(length)?))
        }
        FieldKind::Bytes | FieldKind::Message(_) => {
            let length = reader.uint32()?;
            Value::Delimited(Cow::Borrowed(reader.take(length)?))
        }
    })
}

const BOM: &[u8] = b"\xef\xbb\xbf";

/// UTF-8 as `TextDecoder` reads it: a leading byte order mark dropped, each invalid sequence U+FFFD.
fn text(bytes: &[u8]) -> Cow<'_, [u8]> {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    match String::from_utf8_lossy(bytes) {
        Cow::Borrowed(valid) => Cow::Borrowed(valid.as_bytes()),
        Cow::Owned(replaced) => Cow::Owned(replaced.into_bytes()),
    }
}

fn put_varint(buf: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buf.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

fn put_value(buf: &mut Vec<u8>, number: u32, value: &Value<'_>) {
    let (wire_type, body): (u32, &[u8]) = match value {
        Value::Varint(n) => {
            put_varint(buf, u64::from(number << 3 | VARINT));
            put_varint(buf, *n);
            return;
        }
        Value::Fixed32(bytes) => (FIXED32, bytes),
        Value::Fixed64(bytes) => (FIXED64, bytes),
        Value::Delimited(bytes) => {
            put_varint(buf, u64::from(number << 3 | DELIMITED));
            put_varint(buf, bytes.len() as u64);
            buf.extend_from_slice(bytes);
            return;
        }
    };
    put_varint(buf, u64::from(number << 3 | wire_type));
    buf.extend_from_slice(body);
}

/// A message's fields written back in the order read, a singular field's earlier occurrences left out.
#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
    /// Where each singular field's latest occurrence sits.
    singular: Vec<(u32, usize, usize)>,
    /// Earlier occurrences, overwritten.
    dropped: Vec<(usize, usize)>,
}

impl Writer {
    fn field(&mut self, field: &FieldSchema, write: impl FnOnce(&mut Vec<u8>)) {
        let start = self.buf.len();
        write(&mut self.buf);
        let end = self.buf.len();
        if field.repeated {
            return;
        }
        match self
            .singular
            .iter_mut()
            .find(|(number, _, _)| *number == field.number)
        {
            Some(slot) => {
                self.dropped.push((slot.1, slot.2));
                *slot = (field.number, start, end);
            }
            None => self.singular.push((field.number, start, end)),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.dropped.is_empty() {
            return self.buf;
        }
        self.dropped.sort_unstable();
        let mut out = Vec::with_capacity(self.buf.len());
        let mut at = 0;
        for (start, end) in self.dropped {
            out.extend_from_slice(self.buf.get(at..start).unwrap_or_default());
            at = end;
        }
        out.extend_from_slice(self.buf.get(at..).unwrap_or_default());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(name: &str) -> usize {
        message_index(name).unwrap()
    }

    /// A field's number, as a tag byte's high bits.
    fn number(message: &str, field: &str) -> u8 {
        let fields = MESSAGES[index(message)].fields;
        fields.iter().find(|f| f.name == field).unwrap().number as u8
    }

    fn sanitized(name: &str, bytes: &[u8]) -> Result<Vec<u8>, WireError> {
        sanitize(MESSAGES, index(name), bytes, DecodeLimit::NONE).map(|s| s.bytes)
    }

    #[test]
    fn keeps_canonical_fields_as_they_are() {
        // MessageInfo: type 5, versions 1, 0, 5 packed, length 9.
        let wire = [0x08, 5, 0x12, 3, 1, 0, 5, 0x18, 9];
        assert_eq!(
            sanitized("TSP.MessageInfo", &wire).unwrap(),
            [0x08, 5, 0x10, 1, 0x10, 0, 0x10, 5, 0x18, 9]
        );
    }

    #[test]
    fn reads_packed_and_unpacked_runs_alike_and_drops_unknown_fields_groups_and_other_wire_types() {
        // version (field 2, uint32 repeated): unpacked 1, 2, then a packed run of 3, 4.
        let versions = [0x10, 1, 0x10, 2, 0x12, 2, 3, 4];
        assert_eq!(
            sanitized("TSP.MessageInfo", &versions).unwrap(),
            [0x10, 1, 0x10, 2, 0x10, 3, 0x10, 4]
        );
        // Field 9 (unknown) as a varint, a fixed32, a group holding a varint, then field 1 (type) as a fixed64,
        // then as a varint.
        let skipped = [
            0x48, 0x96, 0x01, 0x4d, 1, 2, 3, 4, 0x4b, 0x08, 7, 0x4c, 0x09, 1, 2, 3, 4, 5, 6, 7, 8,
            0x08, 5,
        ];
        assert_eq!(sanitized("TSP.MessageInfo", &skipped).unwrap(), [0x08, 5]);
    }

    #[test]
    fn keeps_the_last_value_of_a_singular_field_and_replaces_a_message_set_twice() {
        // type 1, then type 2.
        assert_eq!(
            sanitized("TSP.MessageInfo", &[0x08, 1, 0x08, 2]).unwrap(),
            [0x08, 2]
        );
        // ArchiveInfo with message_infos twice (repeated: both kept), then identifier 1 and 2.
        let wire = [0x12, 2, 0x08, 1, 0x08, 1, 0x12, 2, 0x08, 2, 0x08, 2];
        assert_eq!(
            sanitized("TSP.ArchiveInfo", &wire).unwrap(),
            [0x12, 2, 0x08, 1, 0x12, 2, 0x08, 2, 0x08, 2]
        );
        // A singular message: KN.SlideNodeArchive.slide.
        let tag = number("KN.SlideNodeArchive", "slide") << 3 | 2;
        let slide = |id: u8| [tag, 2, 0x08, id];
        let twice: Vec<u8> = [slide(7), slide(8)].concat();
        assert_eq!(sanitized("KN.SlideNodeArchive", &twice).unwrap(), slide(8));
    }

    #[test]
    fn reads_bool_uint32_and_int32_from_the_low_32_bits() {
        // Reference.deprecated_is_external (3, bool) = 2^32: false. deprecated_type (2, int32) = 2^32 - 1: -1.
        let wire = [
            0x18, 0x80, 0x80, 0x80, 0x80, 0x10, 0x10, 0xff, 0xff, 0xff, 0xff, 0x0f,
        ];
        let mut expected = vec![0x18, 0, 0x10];
        expected.extend([0xff; 9]);
        expected.push(1);
        assert_eq!(sanitized("TSP.Reference", &wire).unwrap(), expected);
        // MessageInfo.type (uint32) = 2^32 + 5 reads 5.
        assert_eq!(
            sanitized("TSP.MessageInfo", &[0x08, 0x85, 0x80, 0x80, 0x80, 0x10]).unwrap(),
            [0x08, 5]
        );
    }

    #[test]
    fn decodes_strings_as_text_decoder_does() {
        let name = number("TSS.StyleArchive", "name");
        let wire = |text: &[u8]| {
            let mut out = vec![name << 3 | 2, text.len() as u8];
            out.extend(text);
            out
        };
        // A leading BOM goes, a later one stays; an invalid byte is U+FFFD.
        let input = wire(b"\xef\xbb\xbfa\xef\xbb\xbf\xff");
        assert_eq!(
            sanitized("TSS.StyleArchive", &input).unwrap(),
            wire("a\u{feff}\u{fffd}".as_bytes())
        );
    }

    /// Invalid UTF-8 and what `TextDecoder` reads from it.
    #[test]
    fn replaces_invalid_utf8_as_text_decoder_does() {
        let name = number("TSS.StyleArchive", "name");
        for (input, expected) in [
            ("ff", "efbfbd"),
            ("c080", "efbfbdefbfbd"),
            ("eda080", "efbfbdefbfbdefbfbd"),
            ("f09f98", "efbfbd"),
            ("e28261", "efbfbd61"),
            ("f4908080", "efbfbdefbfbdefbfbdefbfbd"),
            ("efbbbfefbbbf61", "efbbbf61"),
            ("efbbbf", ""),
            ("61efbbbf", "61efbbbf"),
            ("e0a0", "efbfbd"),
            ("f8888080", "efbfbdefbfbdefbfbdefbfbd"),
            ("c2", "efbfbd"),
            ("ed9fbf", "ed9fbf"),
            ("f0908080", "f0908080"),
            ("e080af", "efbfbdefbfbdefbfbd"),
        ] {
            let bytes = |hex: &str| -> Vec<u8> {
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect()
            };
            let wire = |text: Vec<u8>| {
                let mut out = vec![name << 3 | 2, text.len() as u8];
                out.extend(text);
                out
            };
            assert_eq!(
                sanitized("TSS.StyleArchive", &wire(bytes(input))).unwrap(),
                wire(bytes(expected)),
                "{input}"
            );
        }
    }

    #[test]
    fn counts_every_message_it_builds_and_stops_past_its_limit() {
        // A slide naming two drawables: three messages.
        let drawables = number("KN.SlideArchive", "owned_drawables");
        let reference = |id: u8| [drawables << 3 | 2, 2, 0x08, id];
        let payload: Vec<u8> = [reference(7), reference(8)].concat();
        let slide = index("KN.SlideArchive");
        assert_eq!(
            sanitize(MESSAGES, slide, &payload, DecodeLimit::NONE)
                .unwrap()
                .messages,
            3
        );
        let messages = |messages| DecodeLimit {
            messages,
            bytes: u64::MAX,
        };
        assert_eq!(
            sanitize(MESSAGES, slide, &payload, messages(3))
                .unwrap()
                .messages,
            3
        );
        assert_eq!(
            sanitize(MESSAGES, slide, &payload, messages(2)),
            Err(WireError::TooManyMessages(2))
        );
    }

    #[test]
    fn costs_a_message_where_it_stands_alone_and_an_inline_one_in_its_parent() {
        // A slide node with its slide (singular, inline) and two children (repeated).
        let slide = number("KN.SlideNodeArchive", "slide") << 3 | 2;
        let children = number("KN.SlideNodeArchive", "children") << 3 | 2;
        let wire = [
            slide, 2, 0x08, 1, children, 2, 0x08, 2, children, 2, 0x08, 3,
        ];
        let node = index("KN.SlideNodeArchive");
        let decoded = sanitize(MESSAGES, node, &wire, DecodeLimit::NONE).unwrap();
        let cost = |name| MESSAGES[index(name)].cost;
        assert_eq!(decoded.messages, 4);
        assert_eq!(
            decoded.cost,
            cost("KN.SlideNodeArchive") + 2 * cost("TSP.Reference")
        );
        // 24, plus 16 for each of TSP.Reference's three scalar fields.
        assert_eq!(cost("TSP.Reference"), 72);
        let limit = DecodeLimit {
            messages: u64::MAX,
            bytes: decoded.cost - 1,
        };
        assert_eq!(
            sanitize(MESSAGES, node, &wire, limit),
            Err(WireError::TooManyBytes(decoded.cost - 1))
        );
    }

    #[test]
    fn costs_each_element_of_a_repeated_value_and_each_strings_length() {
        static VALUES: [MessageSchema; 1] = [MessageSchema {
            name: "Values",
            cost: 24,
            fields: &[
                FieldSchema {
                    number: 1,
                    name: "numbers",
                    kind: FieldKind::Uint32,
                    repeated: true,
                },
                FieldSchema {
                    number: 2,
                    name: "names",
                    kind: FieldKind::String,
                    repeated: true,
                },
                FieldSchema {
                    number: 3,
                    name: "name",
                    kind: FieldKind::String,
                    repeated: false,
                },
                FieldSchema {
                    number: 4,
                    name: "flags",
                    kind: FieldKind::Bool,
                    repeated: true,
                },
                FieldSchema {
                    number: 5,
                    name: "doubles",
                    kind: FieldKind::Double,
                    repeated: true,
                },
                FieldSchema {
                    number: 6,
                    name: "count",
                    kind: FieldKind::Uint32,
                    repeated: false,
                },
            ],
        }];
        let cost = |wire: &[u8]| sanitize(&VALUES, 0, wire, DecodeLimit::NONE).unwrap().cost;
        // Three numbers packed and one unpacked, the names "" and "ab", the name "xyz", two flags packed, a
        // double and a count.
        let mut wire = vec![0x0a, 3, 1, 2, 3, 0x08, 4];
        wire.extend([0x12, 0, 0x12, 2, b'a', b'b', 0x1a, 3, b'x', b'y', b'z']);
        wire.extend([0x22, 2, 1, 0, 0x29, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0x30, 7]);
        let whole = 24 + 4 * 4 + 24 + (24 + 2) + 3 + 2 + 8;
        assert_eq!(cost(&wire), whole);
        assert_eq!(cost(&[0x30, 7]), 24);
        // A packed run costs its elements, however few bytes each takes.
        let mut run = vec![0x0a];
        put_varint(&mut run, 100_000);
        run.extend(std::iter::repeat_n(1, 100_000));
        assert_eq!(cost(&run), 24 + 4 * 100_000);
        let limit = DecodeLimit {
            messages: u64::MAX,
            bytes: whole - 1,
        };
        assert_eq!(
            sanitize(&VALUES, 0, &wire, limit),
            Err(WireError::TooManyBytes(whole - 1))
        );
    }

    #[test]
    fn refuses_damaged_wire_data() {
        let info = index("TSP.ArchiveInfo");
        for (wire, error) in [
            (&[0x08, 0x80][..], WireError::VarintPastEnd),
            (
                &[
                    0x08, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 1,
                ][..],
                WireError::VarintTooLong,
            ),
            (&[0x1a, 5, 1][..], WireError::FieldPastEnd),
            (&[0x0f][..], WireError::WireType(7)),
            (&[0x0c][..], WireError::WireType(4)),
        ] {
            assert_eq!(
                sanitize(MESSAGES, info, wire, DecodeLimit::NONE),
                Err(error),
                "{wire:?}"
            );
        }
    }

    #[test]
    fn refuses_groups_nested_past_the_depth() {
        let info = index("TSP.ArchiveInfo");
        let mut wire = vec![0x4b; 66];
        wire.extend([0x4c; 66]);
        assert_eq!(
            sanitize(MESSAGES, info, &wire, DecodeLimit::NONE),
            Err(WireError::GroupsTooDeep)
        );
        let mut shallow = vec![0x4b; 65];
        shallow.extend([0x4c; 65]);
        assert!(sanitize(MESSAGES, info, &shallow, DecodeLimit::NONE).is_ok());
    }

    #[test]
    fn refuses_messages_nested_past_the_depth() {
        static NESTED: [MessageSchema; 1] = [MessageSchema {
            name: "Nested",
            cost: 24,
            fields: &[FieldSchema {
                number: 1,
                name: "child",
                kind: FieldKind::Message(0),
                repeated: false,
            }],
        }];
        let nest = |levels: usize| {
            let mut wire: Vec<u8> = Vec::new();
            for _ in 0..levels {
                let mut outer = vec![0x0a];
                put_varint(&mut outer, wire.len() as u64);
                outer.extend(wire);
                wire = outer;
            }
            wire
        };
        assert!(sanitize(&NESTED, 0, &nest(64), DecodeLimit::NONE).is_ok());
        assert_eq!(
            sanitize(&NESTED, 0, &nest(70), DecodeLimit::NONE),
            Err(WireError::MessagesTooDeep)
        );
    }
}
