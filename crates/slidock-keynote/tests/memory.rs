//! What decoding an index holds in memory, measured by a counting allocator: at most a few times what its stats
//! count, so the decode budget bounds it however the bytes amplify.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]
#![allow(
    unsafe_code,
    reason = "the counting allocator passes each call on to the system's"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use slidock_keynote::iwa::schema::{FieldKind, message_index, messages};
use slidock_keynote::iwa::{DecodeLimit, IwaError, IwaStats, read_iwa, type_id};

/// Counts the bytes each thread holds, and the most it held.
struct Counting;

thread_local! {
    static LIVE: Cell<u64> = const { Cell::new(0) };
    static PEAK: Cell<u64> = const { Cell::new(0) };
}

fn grow(bytes: usize) {
    let _ = LIVE.try_with(|live| {
        live.set(live.get() + bytes as u64);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
    });
}

fn shrink(bytes: usize) {
    let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(bytes as u64)));
}

// SAFETY: every call is passed to `System` unchanged; the counters are thread-local cells with no destructor.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, passed on.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, passed on.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(pointer, layout) };
        shrink(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the caller's contract, passed on.
        let moved = unsafe { System.realloc(pointer, layout, size) };
        if !moved.is_null() {
            grow(size);
            shrink(layout.size());
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What `run` returns, the most it held above what was held before, and what it still holds after.
fn measured<T>(run: impl FnOnce() -> T) -> (T, u64, u64) {
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let out = run();
    let peak = PEAK.with(Cell::get) - before;
    let held = LIVE.with(Cell::get).saturating_sub(before);
    (out, peak, held)
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// One object stream record: an ArchiveInfo of `info` (after identifier 1), then `payload`.
fn record(info: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut header = vec![0x08, 1];
    header.extend(info);
    let mut out = Vec::new();
    varint(&mut out, header.len() as u64);
    out.extend(header);
    out.extend(payload);
    out
}

const MIB: u64 = 1024 * 1024;

#[test]
fn an_amplifying_object_header_holds_no_more_than_its_budget() {
    let limit = DecodeLimit {
        messages: u64::MAX,
        bytes: 8 * MIB,
    };
    // 2,000,000 empty MessageInfos: 4 MB of stream, 288 MB as structs.
    let stream = record(&[0x12, 0].repeat(2_000_000), &[]);
    let mut stats = IwaStats::default();
    let (read, peak, _) = measured(|| read_iwa(&stream, "a.iwa", &mut stats, limit));
    assert!(matches!(read, Err(IwaError::TooLarge(_))), "{read:?}");
    let bound = 4 * limit.bytes + 8 * stream.len() as u64;
    assert!(peak <= bound, "held {peak} bytes, at most {bound}");
    // The same bytes as a field the schema leaves out.
    let skipped = record(&[0x78, 0].repeat(2_000_000), &[]);
    let mut stats = IwaStats::default();
    let (read, peak, _) = measured(|| read_iwa(&skipped, "a.iwa", &mut stats, limit));
    assert_eq!(read.unwrap().objects.len(), 0);
    assert!(peak <= 8 * skipped.len() as u64, "held {peak} bytes");
}

#[test]
fn a_packed_run_holds_no_more_than_it_costs() {
    // TSWP.ListStyleArchive.label_types, a repeated enum: 1,000,000 one-byte elements.
    let name = "TSWP.ListStyleArchive";
    let schema = &messages()[message_index(name).unwrap()];
    let field = schema
        .fields
        .iter()
        .find(|f| f.name == "label_types")
        .unwrap();
    assert!(field.repeated && field.kind == FieldKind::Enum);
    let mut payload = Vec::new();
    varint(&mut payload, u64::from(field.number << 3 | 2));
    varint(&mut payload, 1_000_000);
    payload.extend(std::iter::repeat_n(1, 1_000_000));
    let mut info = vec![0x08];
    varint(&mut info, u64::from(type_id(name).unwrap()));
    info.push(0x18);
    varint(&mut info, payload.len() as u64);
    let mut message_info = vec![0x12];
    varint(&mut message_info, info.len() as u64);
    message_info.extend(info);
    let stream = record(&message_info, &payload);
    let mut stats = IwaStats::default();
    let (read, _, held) = measured(|| read_iwa(&stream, "a.iwa", &mut stats, DecodeLimit::NONE));
    let file = read.unwrap();
    assert!(file.objects[0].archive.is_some());
    assert!(
        stats.decoded_bytes >= 4 * 1_000_000,
        "{}",
        stats.decoded_bytes
    );
    assert!(
        held <= 4 * stats.decoded_bytes,
        "holds {held} bytes, counted {}",
        stats.decoded_bytes
    );
}
