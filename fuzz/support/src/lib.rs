//! What the fuzz targets share: a counting allocator, so a target can check that a decode holds no more memory than
//! its stats count, and the Keynote checks of the `proto` target.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use slidock_keynote::iwa::MAX_DEPTH;
use slidock_keynote::iwa::schema::{FieldKind, messages};

/// The system allocator, counting the bytes each thread holds and the most it held. A target installs it with
/// `#[global_allocator]` and measures a call with [`measured`].
pub struct Counting;

thread_local! {
    static LIVE: Cell<u64> = const { Cell::new(0) };
    static PEAK: Cell<u64> = const { Cell::new(0) };
}

fn grow(bytes: usize) {
    let _ = LIVE.try_with(|live| {
        live.set(live.get().saturating_add(bytes as u64));
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
    });
}

fn shrink(bytes: usize) {
    let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(bytes as u64)));
}

// SAFETY: every call is passed on to `System` unchanged; the counters are thread-local cells with no destructor,
// so counting never allocates.
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

/// A call's result and the memory it took on this thread, as [`Counting`] counts it.
#[derive(Debug)]
pub struct Measured<T> {
    pub value: T,
    /// The most it held at once, above what was held before.
    pub peak: u64,
    /// What it still holds after: what `value` owns.
    pub held: u64,
}

/// Runs `run` and measures it; without [`Counting`] installed every count is 0.
pub fn measured<T>(run: impl FnOnce() -> T) -> Measured<T> {
    let before = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let value = run();
    Measured {
        value,
        peak: PEAK.with(Cell::get).saturating_sub(before),
        held: LIVE.with(Cell::get).saturating_sub(before),
    }
}

const BOM: &[u8] = b"\xef\xbb\xbf";

/// Whether a string field anywhere in `bytes`, read as the message `index` of the Keynote schema, starts with a byte
/// order mark (which the pre-pass drops on every read, as `TextDecoder` does). `None` when the bytes are not
/// well-formed protobuf.
pub fn leading_bom(index: usize, bytes: &[u8]) -> Option<bool> {
    walk(index, bytes, 0)
}

fn walk(index: usize, bytes: &[u8], depth: usize) -> Option<bool> {
    if depth > MAX_DEPTH {
        return None;
    }
    let message = messages().get(index)?;
    let mut at = 0;
    while at < bytes.len() {
        let tag = varint(bytes, &mut at)?;
        let number = u32::try_from(tag >> 3).ok()?;
        match tag & 7 {
            0 => {
                varint(bytes, &mut at)?;
            }
            1 => at = at.checked_add(8).filter(|&end| end <= bytes.len())?,
            5 => at = at.checked_add(4).filter(|&end| end <= bytes.len())?,
            2 => {
                let length = usize::try_from(varint(bytes, &mut at)?).ok()?;
                let end = at.checked_add(length)?;
                let body = bytes.get(at..end)?;
                at = end;
                match message.field(number).map(|field| field.kind) {
                    Some(FieldKind::String) if body.starts_with(BOM) => return Some(true),
                    Some(FieldKind::Message(child)) if walk(child, body, depth + 1)? => {
                        return Some(true);
                    }
                    _ => {}
                }
            }
            _ => return None,
        }
    }
    Some(false)
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f).checked_shl(shift).unwrap_or(0);
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_keynote::iwa::schema::message_index;

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    fn field(message: &str, name: &str) -> (usize, u8, FieldKind) {
        let index = message_index(message).unwrap();
        let field = messages()[index]
            .fields
            .iter()
            .find(|f| f.name == name)
            .unwrap();
        (index, field.number as u8, field.kind)
    }

    fn delimited(number: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![number << 3 | 2, body.len() as u8];
        out.extend(body);
        out
    }

    #[test]
    fn finds_a_string_that_starts_with_a_byte_order_mark_at_any_depth() {
        let (style, name, kind) = field("TSS.StyleArchive", "name");
        assert_eq!(kind, FieldKind::String);
        assert_eq!(
            leading_bom(style, &delimited(name, b"\xef\xbb\xbfa")),
            Some(true)
        );
        // Later in the string, or in a string that does not start with it, the mark stays.
        assert_eq!(
            leading_bom(style, &delimited(name, b"a\xef\xbb\xbf")),
            Some(false)
        );
        // Nested: a paragraph style's `super` (a TSS.StyleArchive) named with a leading mark.
        let (paragraph, inner, kind) = field("TSWP.ParagraphStyleArchive", "super");
        assert!(matches!(kind, FieldKind::Message(_)));
        let nested = delimited(inner, &delimited(name, b"\xef\xbb\xbf"));
        assert_eq!(leading_bom(paragraph, &nested), Some(true));
        let plain = delimited(inner, &delimited(name, b"x"));
        assert_eq!(leading_bom(paragraph, &plain), Some(false));
    }

    #[test]
    fn passes_over_the_mark_in_bytes_and_numbers() {
        // The mark in a bytes field.
        let (index, number) = messages()
            .iter()
            .enumerate()
            .find_map(|(index, message)| {
                let field = message.fields.iter().find(|f| f.kind == FieldKind::Bytes)?;
                Some((index, field.number as u8))
            })
            .unwrap();
        assert_eq!(
            leading_bom(index, &delimited(number, b"\xef\xbb\xbf")),
            Some(false)
        );
        // A fixed32 of EF BB BF 00 in TSP.Point.x.
        let (point, x, kind) = field("TSP.Point", "x");
        assert_eq!(kind, FieldKind::Float);
        assert_eq!(
            leading_bom(point, &[x << 3 | 5, 0xef, 0xbb, 0xbf, 0]),
            Some(false)
        );
        assert_eq!(leading_bom(point, &[x << 3 | 5, 0xef]), None);
    }

    #[test]
    fn measures_what_a_call_holds() {
        let measure = measured(|| vec![0u8; 1 << 20]);
        assert_eq!(measure.value.len(), 1 << 20);
        assert!(measure.peak >= 1 << 20 && measure.held >= 1 << 20);
        let measure = measured(|| vec![0u8; 1 << 20].len());
        assert!(measure.peak >= 1 << 20);
        assert!(measure.held < 1024, "{}", measure.held);
    }
}
