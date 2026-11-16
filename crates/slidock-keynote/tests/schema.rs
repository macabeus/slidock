//! The schema-driven decoder end to end: random messages of every type the schema keeps, written with the wire
//! forms Keynote and hostile files use (packed and unpacked runs, a singular field set twice, varints with high bits,
//! unknown fields, groups and fields of another wire type), decode through `read_iwa` and `prost` to the values the
//! reference decoder reads, compared as `iwa::dump` text.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_keynote::iwa::schema::{FieldKind, messages};
use slidock_keynote::iwa::{DecodeLimit, is_kept, unframe_iwa};
use slidock_keynote::iwa::{IwaStats, dump, read_iwa, registry, type_id};
use support::{IwaRecord, Seeded, Type, iwa_file, varint};

#[derive(Clone, Debug)]
enum Value {
    Bool(bool),
    Uint32(u32),
    Int32(i32),
    Uint64(u64),
    Fixed32(u32),
    Float(f32),
    Double(f64),
    Text(String),
    Bytes(Vec<u8>),
    Message(Message),
}

#[derive(Clone, Debug)]
struct Message {
    index: usize,
    /// Set fields by number, each with its values (one for a singular field).
    fields: Vec<(u32, Vec<Value>)>,
}

fn random_value(kind: FieldKind, random: &mut Seeded, depth: usize) -> Value {
    let u32_of = |random: &mut Seeded| (random.next() * 4_294_967_296.0) as u32;
    match kind {
        FieldKind::Bool => Value::Bool(random.next() < 0.5),
        FieldKind::Uint32 => Value::Uint32(u32_of(random)),
        FieldKind::Int32 | FieldKind::Enum => Value::Int32(u32_of(random) as i32),
        FieldKind::Uint64 => {
            Value::Uint64(u64::from(u32_of(random)) << 32 | u64::from(u32_of(random)))
        }
        FieldKind::Fixed32 => Value::Fixed32(u32_of(random)),
        FieldKind::Float => Value::Float(((random.next() - 0.5) * 1e6) as f32),
        FieldKind::Double => Value::Double((random.next() - 0.5) * 1e12),
        FieldKind::String => {
            let chars = ['a', 'é', '€', '😀', '\n'];
            let n = random.below(6);
            Value::Text((0..n).map(|_| chars[random.below(chars.len())]).collect())
        }
        FieldKind::Bytes => Value::Bytes(
            (0..random.below(5))
                .map(|_| random.below(256) as u8)
                .collect(),
        ),
        FieldKind::Message(index) => Value::Message(random_message(index, random, depth - 1)),
    }
}

/// Every field set or left out at random, nesting at most `depth` deep.
fn random_message(index: usize, random: &mut Seeded, depth: usize) -> Message {
    let mut fields = Vec::new();
    for field in messages()[index].fields {
        if random.next() < 0.4 || (matches!(field.kind, FieldKind::Message(_)) && depth == 0) {
            continue;
        }
        let count = if field.repeated {
            1 + random.below(3)
        } else {
            1
        };
        let values = (0..count)
            .map(|_| random_value(field.kind, random, depth))
            .collect();
        fields.push((field.number, values));
    }
    Message { index, fields }
}

fn tag(number: u32, wire_type: u32) -> Vec<u8> {
    varint(u64::from(number << 3 | wire_type))
}

fn delimited(number: u32, body: &[u8]) -> Vec<u8> {
    let mut out = tag(number, 2);
    out.extend(varint(body.len() as u64));
    out.extend(body);
    out
}

/// A varint whose low 32 bits are `low`, with random high bits that the reference decoder ignores.
fn low_bits(low: u32, random: &mut Seeded) -> Vec<u8> {
    let high = if random.next() < 0.3 {
        random.below(1 << 20) as u64
    } else {
        0
    };
    varint(high << 32 | u64::from(low))
}

/// One value's wire bytes without its tag.
fn body(value: &Value, random: &mut Seeded) -> Vec<u8> {
    match value {
        Value::Bool(b) => {
            // True is any nonzero low word.
            let low = if *b { 1 + random.below(1000) as u32 } else { 0 };
            low_bits(low, random)
        }
        Value::Uint32(n) => low_bits(*n, random),
        Value::Int32(n) => {
            if random.next() < 0.5 {
                varint(i64::from(*n) as u64)
            } else {
                low_bits(*n as u32, random)
            }
        }
        Value::Uint64(n) => varint(*n),
        Value::Fixed32(n) => n.to_le_bytes().to_vec(),
        Value::Float(n) => n.to_le_bytes().to_vec(),
        Value::Double(n) => n.to_le_bytes().to_vec(),
        Value::Text(text) => {
            let mut out = varint(text.len() as u64);
            out.extend(text.as_bytes());
            out
        }
        Value::Bytes(bytes) => {
            let mut out = varint(bytes.len() as u64);
            out.extend(bytes);
            out
        }
        Value::Message(message) => {
            let inner = encode(message, random);
            let mut out = varint(inner.len() as u64);
            out.extend(inner);
            out
        }
    }
}

fn wire_type(kind: FieldKind) -> u32 {
    match kind {
        FieldKind::Bool
        | FieldKind::Uint32
        | FieldKind::Int32
        | FieldKind::Enum
        | FieldKind::Uint64 => 0,
        FieldKind::Fixed32 | FieldKind::Float => 5,
        FieldKind::Double => 1,
        FieldKind::String | FieldKind::Bytes | FieldKind::Message(_) => 2,
    }
}

/// Bytes a decoder skips: a field the schema lacks, a group, or a known field of another wire type.
fn noise(number: Option<u32>, kind: Option<FieldKind>, random: &mut Seeded) -> Vec<u8> {
    let number = number.unwrap_or(50_000 + random.below(1000) as u32);
    let mut choices = vec![0u32, 1, 2, 3, 5];
    if let Some(kind) = kind {
        let own = wire_type(kind);
        // A repeated number's run is read packed, so a known field only gets wire types it cannot be read with.
        choices.retain(|&w| w != own && w != 2);
    }
    match choices[random.below(choices.len())] {
        0 => [tag(number, 0), varint(random.below(1 << 30) as u64)].concat(),
        1 => [tag(number, 1), vec![7; 8]].concat(),
        2 => delimited(number, &[1, 2, 3]),
        3 => [tag(number, 3), tag(1, 0), vec![5], tag(number, 4)].concat(),
        _ => [tag(number, 5), vec![9; 4]].concat(),
    }
}

fn encode(message: &Message, random: &mut Seeded) -> Vec<u8> {
    let schema = messages()[message.index];
    let mut out = Vec::new();
    for (number, values) in &message.fields {
        let field = schema.field(*number).unwrap();
        if random.next() < 0.15 {
            out.extend(noise(None, None, random));
        }
        if random.next() < 0.15 {
            out.extend(noise(Some(*number), Some(field.kind), random));
        }
        let numeric = !matches!(
            field.kind,
            FieldKind::String | FieldKind::Bytes | FieldKind::Message(_)
        );
        if !field.repeated && random.next() < 0.2 {
            // A value the next one overwrites; a message is replaced, not merged.
            let decoy = random_value(field.kind, random, 1);
            out.extend(tag(*number, wire_type(field.kind)));
            out.extend(body(&decoy, random));
        }
        let mut at = 0;
        while at < values.len() {
            if field.repeated && numeric && random.next() < 0.5 {
                let run = 1 + random.below(values.len() - at);
                let packed: Vec<u8> = values[at..at + run]
                    .iter()
                    .flat_map(|v| body(v, random))
                    .collect();
                out.extend(delimited(*number, &packed));
                at += run;
            } else {
                out.extend(tag(*number, wire_type(field.kind)));
                out.extend(body(&values[at], random));
                at += 1;
            }
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn text(value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Uint32(n) | Value::Fixed32(n) => n.to_string(),
        Value::Int32(n) => n.to_string(),
        Value::Uint64(n) => format!("u{n}"),
        Value::Float(n) => slidock_canon::to_js_string(f64::from(*n)),
        Value::Double(n) => slidock_canon::to_js_string(*n),
        Value::Text(s) => format!("s{}", hex(s.as_bytes())),
        Value::Bytes(b) => format!("b{}", hex(b)),
        Value::Message(m) => expected(m),
    }
}

/// The dump of what the reference decoder reads from a message.
fn expected(message: &Message) -> String {
    let schema = messages()[message.index];
    let mut fields = message.fields.clone();
    fields.sort_by_key(|(number, _)| *number);
    let parts: Vec<String> = fields
        .iter()
        .map(|(number, values)| {
            let field = schema.field(*number).unwrap();
            let values: Vec<String> = values.iter().map(text).collect();
            if field.repeated {
                format!("{number}:[{}]", values.join(","))
            } else {
                format!("{number}:{}", values[0])
            }
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

#[test]
fn decodes_random_messages_of_every_kept_type_as_the_reference_decoder_does() {
    let mut random = Seeded::new(5);
    let kept: Vec<(u32, &str)> = registry().filter(|(id, _)| is_kept(*id)).collect();
    let mut names: Vec<&str> = kept.iter().map(|(_, name)| *name).collect();
    names.dedup();
    assert!(names.len() >= 40, "{}", names.len());
    let mut checked = 0;
    for (type_id, name) in kept {
        let index = slidock_keynote::iwa::schema::message_index(name).unwrap();
        for i in 0..20 {
            let message = random_message(index, &mut random, 3);
            let payload = encode(&message, &mut random);
            let file = iwa_file(&[IwaRecord::raw(1, Type::Id(type_id), payload)]);
            let data = unframe_iwa(&file, "a.iwa", u64::MAX).unwrap();
            let mut stats = IwaStats::default();
            let iwa = read_iwa(&data, "a.iwa", &mut stats, DecodeLimit::NONE).unwrap();
            let archive = iwa.objects[0]
                .archive
                .as_ref()
                .unwrap_or_else(|| panic!("{name} #{i} did not decode"));
            assert_eq!(archive.type_name(), name);
            assert_eq!(dump(archive).unwrap(), expected(&message), "{name} #{i}");
            checked += 1;
        }
    }
    assert!(checked >= 800);
}

#[test]
fn a_registered_type_the_schema_leaves_out_is_named_and_skipped() {
    let id = type_id("TSCE.CalculationEngineArchive").unwrap();
    assert!(!is_kept(id));
    assert!(is_kept(type_id("KN.SlideArchive").unwrap()));
    // Two type ids, one message.
    assert_eq!(slidock_keynote::iwa::type_name(5), Some("KN.SlideArchive"));
    assert_eq!(slidock_keynote::iwa::type_name(6), Some("KN.SlideArchive"));
    assert_eq!(type_id("KN.SlideArchive"), Some(5));
    assert_eq!(slidock_keynote::iwa::type_name(999_999), None);
}
