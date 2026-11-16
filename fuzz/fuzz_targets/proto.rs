//! The protobuf pre-pass on every type the trimmed schema decodes: the input's first two bytes pick a kept type,
//! the rest is one payload of it, framed as an object (an ArchiveInfo header naming the type and length) and read
//! through the object stream. A payload that decodes dumps, and its canonical bytes decode to the same object. The
//! limit is scaled to the inputs, so they reach it, and what the decode holds in memory stays within a few times
//! what its stats count.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, leading_bom, measured};
use slidock_keynote::iwa::schema::message_index;
use slidock_keynote::iwa::{DecodeLimit, IwaStats, dump, is_kept, read_iwa, registry, type_name};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const FILE: &str = "Index/Fuzz.iwa";

/// Small enough for inputs of `-max_len` (16 KiB) to reach: a few thousand empty elements of a repeated message field.
const LIMIT: DecodeLimit = DecodeLimit {
    messages: 20_000,
    bytes: 1024 * 1024,
};

fn kept() -> &'static [u32] {
    static KEPT: OnceLock<Vec<u32>> = OnceLock::new();
    KEPT.get_or_init(|| {
        registry()
            .map(|(id, _)| id)
            .filter(|&id| is_kept(id))
            .collect()
    })
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// One object of `type_id` holding `payload`.
fn object(type_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut info = Vec::new();
    varint(&mut info, u64::from(type_id));
    info.insert(0, 0x08);
    info.push(0x18);
    varint(&mut info, payload.len() as u64);
    let mut header = vec![0x08, 1, 0x12];
    varint(&mut header, info.len() as u64);
    header.extend(info);
    let mut out = Vec::new();
    varint(&mut out, header.len() as u64);
    out.extend(header);
    out.extend_from_slice(payload);
    out
}

fuzz_target!(|data: &[u8]| {
    let [a, b, payload @ ..] = data else {
        return;
    };
    let kept = kept();
    let Some(&type_id) = kept.get(usize::from(u16::from_le_bytes([*a, *b])) % kept.len().max(1))
    else {
        return;
    };
    let stream = object(type_id, payload);
    let mut stats = IwaStats::default();
    let read = measured(|| read_iwa(&stream, FILE, &mut stats, LIMIT));
    let counted = match &read.value {
        Ok(_) => stats.decoded_bytes,
        Err(_) => LIMIT.bytes,
    };
    let bound = 8 * counted + 48 * stream.len() as u64 + (1 << 20);
    assert!(
        read.peak <= bound,
        "the decode held {} bytes; it counted {counted}",
        read.peak
    );
    let Ok(file) = read.value else {
        return;
    };
    assert!(
        read.held <= 4 * stats.decoded_bytes + (64 << 10),
        "the object holds {} bytes; it counted {}",
        read.held,
        stats.decoded_bytes
    );
    assert!(stats.messages <= LIMIT.messages && stats.decoded_bytes <= LIMIT.bytes);
    let [decoded] = file.objects.as_slice() else {
        panic!("one object in, {} out", file.objects.len());
    };
    assert_eq!(decoded.type_id, Some(type_id));
    let Some(archive) = &decoded.archive else {
        assert_eq!(stats.decode_errors, 1);
        return;
    };
    let text = dump(archive).unwrap_or_else(|error| panic!("the object does not dump: {error}"));
    // The canonical bytes `prost` writes back decode to the same object (compared by dump: a NaN is unequal). A
    // string that starts with a byte order mark loses it again, as `TextDecoder` drops it, so those are left out.
    let canonical = archive.encode_to_vec();
    let index = type_name(type_id).and_then(message_index);
    let Some(index) = index else {
        panic!("type {type_id} is kept but not in the schema");
    };
    match leading_bom(index, &canonical) {
        Some(true) => return,
        Some(false) => {}
        None => panic!("the canonical bytes are not protobuf"),
    }
    let mut again = IwaStats::default();
    let file = read_iwa(&object(type_id, &canonical), FILE, &mut again, LIMIT)
        .unwrap_or_else(|error| panic!("canonical bytes refused: {error}"));
    let redecoded = file.objects.first().and_then(|o| o.archive.as_ref());
    assert_eq!(
        redecoded.map(|archive| dump(archive).ok()),
        Some(Some(text))
    );
});
